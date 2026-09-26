// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.
//
// Rasterization Shader - Phong Lighting Model + modern-look stack (KE-0401)
//
// This shader implements traditional GPU rasterization with Blinn-Phong lighting,
// plus the KE-0401 "modern look" stack that lives entirely below the render seam:
//
//   * Linear-space lighting presented through an explicit sRGB encode + ACES
//     tonemap (colors correct, not washed out) — see `present_color`.
//   * A gradient sky drawn as a fullscreen pass (`sky_vertex_main` /
//     `sky_fragment_main`) instead of a flat clear.
//   * Distance fog that blends far geometry into the sky/horizon color, hiding
//     the streaming spawn edge (`apply_fog`).
//   * A cheap directional contact-shadow "blob" projected onto the ground plane
//     to ground the car (`ground_shadow` / the shadow term in the lit shaders).
//
// Bloom is deferred for this pass (see KE-0401 report): it needs several extra
// half-res offscreen targets + ping-pong pipelines that could not be validated
// in this environment. The MSAA / resolve wiring is all on the Rust side
// (`backend.rs`); this file only provides the per-fragment math. Both the
// untextured (`vertex_main`/`fragment_main`) and textured
// (`textured_vertex_main`/`textured_fragment_main`) pipelines run the same
// lighting + fog + shadow + present math, so the two pixel-hash reference scenes
// change together and are re-blessed together.

#include <metal_stdlib>
using namespace metal;

// ============================================================================
// Vertex Input/Output Structures
// ============================================================================

/// Vertex data input from CPU-side mesh buffers
struct VertexIn {
    float3 position [[attribute(0)]];  // World-space vertex position
    float3 normal [[attribute(1)]];    // Surface normal for lighting calculations
    float3 color [[attribute(2)]];     // Per-vertex color (RGB)
};

/// Vertex shader output / Fragment shader input
struct VertexOut {
    float4 position [[position]];      // Clip-space position (required for rasterization)
    float3 worldNormal;                // Interpolated normal for fragment shading
    float3 worldPosition;              // World-space position for lighting/fog/shadow
    float3 color;                      // Interpolated vertex color
    float  viewDepth;                  // Positive view-space distance for fog
};

// ============================================================================
// Uniform Structures
// ============================================================================

/// Transformation matrices passed from CPU per draw call
struct Uniforms {
    float4x4 modelViewProjection;      // Combined model-view-projection matrix
    float4x4 model;                    // Model matrix (world-space pos/normal for fog + shadow)
};

/// Lighting parameters (directional light with Phong components) plus the
/// KE-0401 look parameters (fog, sky, shadow). Mirrors the Rust `LightUniforms`.
struct Light {
    float3 direction;                  // Light direction in world space (points FROM the light)
    float3 color;                      // Light color (RGB)
    float ambientIntensity;            // Ambient light strength (constant illumination)
    float diffuseIntensity;            // Diffuse reflection strength (angle-dependent)
    float specularIntensity;           // Specular highlight strength (view-dependent)
    float shininess;                   // Phong exponent (controls highlight tightness)

    // --- KE-0401 look params ---
    float3 skyTopColor;                // Linear zenith color of the gradient sky
    float3 skyHorizonColor;            // Linear horizon color (fog blends toward this)
    float  fogDensity;                 // Exponential fog density (0 disables fog)
    float  fogStart;                   // View distance at which fog begins
    float  fogHeight;                  // World Y at/below which fog is full strength
    float  fogFalloff;                 // e-fold the fog thins over above fogHeight (0 = no falloff)
    float3 shadowCenter;               // World-space point the car sits above (blob center)
    float  shadowRadius;               // Blob shadow radius in world units
    float  shadowStrength;             // 0..1 darkening under the car
    float  groundHeight;               // World Y of the ground plane (shadow receiver)
};

// ============================================================================
// Color management: linear -> ACES tonemap -> sRGB encode (KE-0401)
// ============================================================================

/// Narkowicz ACES filmic tonemap approximation, operating on linear HDR color.
inline float3 aces_tonemap(float3 x) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), 0.0, 1.0);
}

/// Encode a linear color to sRGB (per-channel gamma). Both render targets are
/// *Unorm* (not `_sRGB`), so we encode explicitly here; the offscreen readback
/// then sees perceptually-correct bytes.
inline float3 linear_to_srgb(float3 c) {
    float3 lo = c * 12.92;
    float3 hi = 1.055 * pow(max(c, 1e-5), float3(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= 0.0031308);
}

/// Full present transform: tonemap a linear HDR color and sRGB-encode it for
/// output. Every lit/sky/composite fragment ends with this so colors are
/// consistent and not washed out.
inline float3 present_color(float3 linearHdr) {
    return linear_to_srgb(aces_tonemap(linearHdr));
}

// ============================================================================
// Distance fog (KE-0401)
// ============================================================================

/// Exponential-squared distance fog blending `color` toward the horizon/sky
/// color, **attenuated by world height**. Returns linear color (present happens
/// afterward). Hides the streaming spawn edge by fading far geometry into the sky,
/// while the height falloff keeps the fog hugging the ground so tall geometry (the
/// skyline backdrop, high buildings) stays readable above it.
inline float3 apply_fog(float3 color, float viewDist, float worldY, constant Light& light) {
    if (light.fogDensity <= 0.0) {
        return color;
    }
    float d = max(viewDist - light.fogStart, 0.0) * light.fogDensity;
    float fog = 1.0 - exp(-d * d);

    // Height falloff: full strength at/below fogHeight, thinning upward.
    if (light.fogFalloff > 0.0) {
        fog *= exp(-max(worldY - light.fogHeight, 0.0) / light.fogFalloff);
    }

    fog = clamp(fog, 0.0, 1.0);
    return mix(color, light.skyHorizonColor, fog);
}

/// Cheap directional contact shadow: darken a fragment that sits within
/// `shadowRadius` of the car's ground-projected center. Used to ground the car
/// without a full shadow-map pass. Returns a 0..1 multiplier (1 = fully lit).
inline float ground_shadow(float3 worldPos, constant Light& light) {
    if (light.shadowStrength <= 0.0 || light.shadowRadius <= 0.0) {
        return 1.0;
    }
    float2 d = worldPos.xz - light.shadowCenter.xz;
    float dist = length(d) / light.shadowRadius;
    // Soft falloff, strongest at the center.
    float occ = 1.0 - smoothstep(0.0, 1.0, dist);
    return 1.0 - light.shadowStrength * occ;
}

// ============================================================================
// Vertex Shader
// ============================================================================

/// Transform vertices to clip space and pass data to fragment shader
vertex VertexOut vertex_main(VertexIn in [[stage_in]],
                             constant Uniforms& uniforms [[buffer(1)]]) {
    VertexOut out;

    float4 clip = uniforms.modelViewProjection * float4(in.position, 1.0);
    out.position = clip;

    float4 world = uniforms.model * float4(in.position, 1.0);
    out.worldPosition = world.xyz;
    // Normal transformed by the model matrix (uniform scale assumed for boxes).
    out.worldNormal = (uniforms.model * float4(in.normal, 0.0)).xyz;
    out.color = in.color;
    // Clip.w is the positive view-space depth for our right-handed projection.
    out.viewDepth = clip.w;

    return out;
}

// ============================================================================
// Fragment Shader
// ============================================================================

/// Blinn-Phong lit color in LINEAR space, shared by the lit fragment shaders.
inline float3 lit_linear(float3 albedo, float3 worldNormal, float3 worldPos,
                         float viewDepth, constant Light& light) {
    float3 normal = normalize(worldNormal);
    float3 lightDir = normalize(-light.direction);

    float3 ambient = light.ambientIntensity * light.color;
    float diff = max(dot(normal, lightDir), 0.0);
    float3 diffuse = light.diffuseIntensity * diff * light.color;

    float3 viewDir = normalize(float3(0.0, 0.0, 1.0));
    float3 halfDir = normalize(lightDir + viewDir);
    float spec = pow(max(dot(normal, halfDir), 0.0), light.shininess);
    float3 specular = light.specularIntensity * spec * light.color;

    float shadow = ground_shadow(worldPos, light);
    float3 lit = (ambient + (diffuse + specular) * shadow) * albedo;
    return apply_fog(lit, viewDepth, worldPos.y, light);
}

/// Calculate per-pixel lighting using Blinn-Phong shading model (untextured).
fragment float4 fragment_main(VertexOut in [[stage_in]],
                              constant Light& light [[buffer(0)]]) {
    float3 lit = lit_linear(in.color, in.worldNormal, in.worldPosition,
                            in.viewDepth, light);
    return float4(present_color(lit), 1.0);
}

// ============================================================================
// Gradient sky (KE-0401)
// ============================================================================
//
// A fullscreen triangle drawn first (depth test disabled) that paints a vertical
// gradient from `skyHorizonColor` up to `skyTopColor`, replacing the flat clear.
// Far geometry fogs toward `skyHorizonColor`, so the horizon reads seamlessly.

struct SkyOut {
    float4 position [[position]];
    float2 ndc;                        // -1..1 clip position, for the vertical gradient
};

/// Fullscreen triangle (three verts covering the viewport, no vertex buffer).
vertex SkyOut sky_vertex_main(uint vid [[vertex_id]]) {
    // Oversized triangle covering the whole clip volume.
    float2 pos[3] = { float2(-1.0, -3.0), float2(-1.0, 1.0), float2(3.0, 1.0) };
    SkyOut out;
    out.position = float4(pos[vid], 1.0, 1.0);   // z=1 -> far plane, behind everything
    out.ndc = pos[vid];
    return out;
}

/// Vertical gradient sky, tonemapped + sRGB-encoded like the rest of the scene.
fragment float4 sky_fragment_main(SkyOut in [[stage_in]],
                                  constant Light& light [[buffer(0)]]) {
    float t = clamp(in.ndc.y * 0.5 + 0.5, 0.0, 1.0);
    // Smooth the horizon->zenith transition a touch.
    t = t * t * (3.0 - 2.0 * t);
    float3 sky = mix(light.skyHorizonColor, light.skyTopColor, t);
    return float4(present_color(sky), 1.0);
}

// ============================================================================
// Textured pipeline (KE-0403)
// ============================================================================
//
// A second pipeline reading the `[pos,normal,uv]` layout. It samples the bound
// base-color texture at the interpolated UV (trilinear, mipmapped) and runs the
// same lighting + fog + shadow + present math as the untextured path.

/// Textured vertex input: position, normal, and UV (no per-vertex color).
struct TexturedVertexIn {
    float3 position [[attribute(0)]];
    float3 normal [[attribute(1)]];
    float2 uv [[attribute(2)]];
};

/// Textured vertex output / fragment input.
struct TexturedVertexOut {
    float4 position [[position]];
    float3 worldNormal;
    float3 worldPosition;
    float2 uv;
    float  viewDepth;
};

/// Per-draw material parameters for the textured pipeline (base-color factor).
/// Mirrors the Rust `MaterialUniforms` (`[[buffer(2)]]`), 16 bytes.
struct MaterialUniforms {
    float4 baseColorFactor;            // Linear RGBA, multiplied with the texture
};

/// Textured vertex shader: transform to clip space, pass world data + UV through.
vertex TexturedVertexOut textured_vertex_main(TexturedVertexIn in [[stage_in]],
                                              constant Uniforms& uniforms [[buffer(1)]]) {
    TexturedVertexOut out;
    float4 clip = uniforms.modelViewProjection * float4(in.position, 1.0);
    out.position = clip;
    float4 world = uniforms.model * float4(in.position, 1.0);
    out.worldPosition = world.xyz;
    out.worldNormal = (uniforms.model * float4(in.normal, 0.0)).xyz;
    out.uv = in.uv;
    out.viewDepth = clip.w;
    return out;
}

/// Textured fragment shader: sample the bound base-color texture (trilinear) at
/// the UV, modulate by the material base-color factor, and light/fog/present.
fragment float4 textured_fragment_main(TexturedVertexOut in [[stage_in]],
                                       constant Light& light [[buffer(0)]],
                                       constant MaterialUniforms& material [[buffer(2)]],
                                       texture2d<float> baseColor [[texture(0)]],
                                       sampler baseColorSampler [[sampler(0)]]) {
    float4 sampled = baseColor.sample(baseColorSampler, in.uv);
    float3 albedo = sampled.rgb * material.baseColorFactor.rgb;

    float3 lit = lit_linear(albedo, in.worldNormal, in.worldPosition,
                            in.viewDepth, light);
    return float4(present_color(lit), sampled.a * material.baseColorFactor.a);
}
