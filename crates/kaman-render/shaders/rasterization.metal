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
//     `sky_fragment_main`) instead of a flat clear, carrying the **sun disc and
//     its glow** (KE-0406) at the direction the light actually comes from.
//   * Distance fog that blends far geometry into the sky/horizon color, hiding
//     the streaming spawn edge (`apply_fog`).
//   * Real sun shadows (KE-0407): a depth-only pass from the sun
//     (`shadow_vertex_main`) renders every caster into a fitted shadow map
//     *before* the scene pass, and both lit pipelines look themselves up in it
//     with receiver-plane-biased, tent-filtered 4x4 PCF (`shadow_visibility`). The
//     fitting, stability and bias reasoning live in `src/shadow.rs`.
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
    float4x4 model;                    // Model matrix (world pos/normal for fog + shadow lookups; places casters)
};

/// Lighting parameters (directional light with Phong components) plus the
/// KE-0401 look parameters (fog, sky), the KE-0407 shadow-map parameters, the
/// KE-0406 camera block and the KE-0407 light matrix.
/// Mirrors the Rust `LightUniforms` **byte for byte** — 256 bytes.
///
/// LAYOUT WARNING: an MSL `float3` occupies 16 bytes, so each one below sits at a
/// 16-byte-aligned offset (0, 16, 48, 64, 112) and the two trailing `float4x4`s at
/// 128 and 192. The Rust side spells those pads out explicitly and asserts every offset;
/// a mismatch here does not fail to compile, it silently shades with the wrong
/// bytes (it has happened once — see the `REFERENCE_HASH` re-bless note).
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

    // --- KE-0407 shadow map (see src/shadow.rs for the reasoning) ---
    float  shadowDepthBias;            // Constant bias, map depth units (0.02 world / depth range)
    float  shadowReceiverSlopeCap;     // Cap on the receiver plane's per-texel depth gradient, map depth units
    float  shadowDistance;             // View depth the map covers; shadows fade out before it
    float  shadowStrength;             // 1 = shadowed fragments keep only sky fill, 0 = off

    // --- KE-0406 camera block (the frame's camera, not its light) ---
    float3 cameraPosition;             // Camera world position -> view-dependent specular
    float4x4 inverseViewProjection;    // clip -> world, for the sky pass's view ray

    // --- KE-0407 ---
    float4x4 lightViewProjection;      // world -> shadow-map clip (fitted ortho * light view)
};

// ============================================================================
// Sun disc + glow tuning (KE-0406)
// ============================================================================
//
// Angular sizes, stored as cosines so the fragment shader only ever needs one dot
// product. The real sun subtends ~0.53 degrees, which is a couple of pixels and
// reads as a stray dot, so the disc is drawn a little larger than life and the
// glow carries most of the impression of brightness.
//
// The glow extent is deliberately narrow (18 degrees). It has to be: the
// screen-space overlay reference renders with an identity view-projection, whose
// unprojection gives every pixel the constant view ray (0,0,1) — and the default
// sun sits 75 degrees off that, so a glow this tight contributes *exactly* zero
// there and cannot move OVERLAY_REFERENCE_HASH.

/// cos of the disc's solid core (~0.45 degrees).
constant float SUN_DISC_COS_INNER = 0.99997;
/// cos of the disc's outer rim (~0.95 degrees); the core fades to 0 by here.
constant float SUN_DISC_COS_OUTER = 0.99986;
/// cos of the glow's outer extent (~18 degrees).
constant float SUN_GLOW_COS = 0.95106;
/// How much brighter than the sun colour the disc's core is. Well over 1 so it
/// clips to white through the ACES tonemap, like looking at a real sun.
constant float SUN_DISC_GAIN = 12.0;
/// Peak brightness of the glow, as a fraction of the sun colour.
constant float SUN_GLOW_GAIN = 0.55;

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

// ============================================================================
// Shadow map (KE-0407)
// ============================================================================
//
// Pass order: `shadow_vertex_main` renders every caster depth-only into the map
// first (its own render pass, stored — never memoryless — so it survives to be
// sampled), then the scene pass samples it here. All tuning constants and their
// justification live in `src/shadow.rs`; the per-frame values arrive in `Light`.

/// Fraction of `shadowDistance` over which shadows fade out (mirrors
/// `SHADOW_FADE_FRACTION` in `src/shadow.rs`).
constant float SHADOW_FADE_FRACTION = 0.2;

/// Depth-only caster input: the position is the only attribute the shadow pass
/// reads, at location 0 in both the `[pos,normal,color]` and `[pos,normal,uv]`
/// layouts (the pipeline's vertex descriptor supplies each layout's stride).
struct ShadowVertexIn {
    float3 position [[attribute(0)]];
};

/// Depth-only caster output: clip position only — there is no fragment stage.
struct ShadowVertexOut {
    float4 position [[position]];
};

/// The shadow pass's minimal vertex function: model -> world -> light clip. No
/// normal, colour or UV plumbing, and no fragment function at all.
vertex ShadowVertexOut shadow_vertex_main(ShadowVertexIn in [[stage_in]],
                                          constant Uniforms& uniforms [[buffer(1)]],
                                          constant Light& light [[buffer(2)]]) {
    ShadowVertexOut out;
    out.position = light.lightViewProjection * (uniforms.model * float4(in.position, 1.0));
    return out;
}

/// How much of the sun reaches `worldPos`: 1 = fully lit, 0 = fully shadowed.
///
/// Filtering: tent-weighted PCF over the 4x4 texels around the lookup — the
/// same weights as a 3x3 grid of bilinear taps one texel apart, i.e. a smooth
/// ~4-texel penumbra — done as explicit per-texel compares so each texel can be
/// compared at its own centre. Bias, per texel:
///
///  * **Receiver plane** — each texel is compared against the depth the
///    receiver's own triangle has *at that texel's centre*, extrapolated along the
///    triangle's plane. The plane comes from the screen-space derivatives of the
///    shadow-map position (exact for a planar triangle), so a lit surface never
///    shadows itself however steeply it faces away from the sun. The per-texel
///    gradient is capped at `shadowReceiverSlopeCap` so an edge-on receiver (whose
///    gradient is unbounded) cannot extrapolate absurdly.
///  * **`shadowDepthBias`** — 2 cm of world, for float error.
///
/// Anything outside the map, or past `shadowDistance`, is lit.
inline float shadow_visibility(float3 worldPos, float viewDepth, constant Light& light,
                               depth2d<float> shadowMap) {
    float4 lightClip = light.lightViewProjection * float4(worldPos, 1.0);
    float3 ndc = lightClip.xyz / lightClip.w;          // orthographic: w == 1
    // Shadow-map (u, v, depth). NDC +Y is up; texture v runs down.
    float3 q = float3(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z);

    // Derivatives first, in uniform control flow (before any per-pixel branch).
    float3 qdx = dfdx(q);
    float3 qdy = dfdy(q);

    if (light.shadowStrength <= 0.0) {
        return 1.0;
    }
    if (any(q.xy < 0.0) || any(q.xy > 1.0) || q.z > 1.0 || q.z < 0.0) {
        return 1.0;
    }

    // Receiver plane in (u, v, depth): its normal is the cross of two in-plane
    // screen derivatives; depth's gradient over (u, v) follows from it.
    float size = float(shadowMap.get_width());
    float3 n = cross(qdx, qdy);
    float nLen = length(n);
    float2 depthPerUv = float2(0.0);
    if (nLen > 0.0) {
        // Edge-on to the light, n.z -> 0 and the gradient explodes: floor the
        // divisor, then cap the per-texel gradient.
        float nz = n.z >= 0.0 ? max(n.z, 1e-6 * nLen) : min(n.z, -1e-6 * nLen);
        depthPerUv = -n.xy / nz;
    }
    float capPerUv = light.shadowReceiverSlopeCap * size;
    depthPerUv = clamp(depthPerUv, -capPerUv, capPerUv);
    float reference = q.z - light.shadowDepthBias;

    // Texel-centre space: texel k's centre sits at k + 0.5 texels.
    float2 pos = q.xy * size - 0.5;
    float2 base = floor(pos);
    float2 f = pos - base;
    // 1D weights of three bilinear taps at -1, 0, +1 texels over texels
    // base-1 .. base+2 (each row sums to 3; the 2D total is 9).
    float wx[4] = { 1.0 - f.x, 1.0, 1.0, f.x };
    float wy[4] = { 1.0 - f.y, 1.0, 1.0, f.y };
    int maxIndex = int(size) - 1;

    float lit = 0.0;
    for (int j = 0; j < 4; ++j) {
        for (int i = 0; i < 4; ++i) {
            float2 texelIndex = base + float2(i - 1, j - 1);
            float2 centreUv = (texelIndex + 0.5) / size;
            float receiver = reference + dot(depthPerUv, centreUv - q.xy);
            uint2 coord = uint2(clamp(int2(texelIndex), int2(0), int2(maxIndex)));
            float occluder = shadowMap.read(coord);
            lit += wx[i] * wy[j] * (receiver <= occluder ? 1.0 : 0.0);
        }
    }
    lit *= (1.0 / 9.0);

    // Fade out over the last stretch of the fitted slab (inside the fog), so the
    // map's far edge never shows as a line.
    float fadeStart = light.shadowDistance * (1.0 - SHADOW_FADE_FRACTION);
    lit = mix(lit, 1.0, smoothstep(fadeStart, light.shadowDistance, viewDepth));

    return mix(1.0, lit, light.shadowStrength);
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
/// `shadow` (from `shadow_visibility`) scales the sun's direct light only; the
/// ambient sky fill reaches shadowed surfaces unchanged.
inline float3 lit_linear(float3 albedo, float3 worldNormal, float3 worldPos,
                         float viewDepth, float shadow, constant Light& light) {
    float3 normal = normalize(worldNormal);
    float3 lightDir = normalize(-light.direction);

    float3 ambient = light.ambientIntensity * light.color;
    float diff = max(dot(normal, lightDir), 0.0);
    float3 diffuse = light.diffuseIntensity * diff * light.color;

    // View-dependent specular (KE-0406): the eye position comes across the seam
    // each frame, so highlights track the camera. This used to be a constant
    // float3(0,0,1), which pinned every highlight to a fixed screen direction —
    // most visible on car bodywork under a low sun, where the highlight should
    // sweep as you drive past it and simply did not.
    float3 viewDir = normalize(light.cameraPosition - worldPos);
    float3 halfDir = normalize(lightDir + viewDir);
    float spec = pow(max(dot(normal, halfDir), 0.0), light.shininess);
    // Only lit faces can have a highlight. With a real view vector a face turned
    // away from the sun but toward the camera would otherwise pick up a spurious
    // one; the CPU mirror in `raytracer.rs` already gates on `ndotl > 0`.
    spec *= step(1e-4, diff);
    float3 specular = light.specularIntensity * spec * light.color;

    float3 lit = (ambient + (diffuse + specular) * shadow) * albedo;
    return apply_fog(lit, viewDepth, worldPos.y, light);
}

/// Calculate per-pixel lighting using Blinn-Phong shading model (untextured).
fragment float4 fragment_main(VertexOut in [[stage_in]],
                              constant Light& light [[buffer(0)]],
                              depth2d<float> shadowMap [[texture(1)]]) {
    float shadow = shadow_visibility(in.worldPosition, in.viewDepth, light, shadowMap);
    float3 lit = lit_linear(in.color, in.worldNormal, in.worldPosition,
                            in.viewDepth, shadow, light);
    return float4(present_color(lit), 1.0);
}

// ============================================================================
// Gradient sky (KE-0401)
// ============================================================================
//
// A fullscreen triangle drawn first (depth test disabled) that paints a vertical
// gradient from `skyHorizonColor` up to `skyTopColor`, replacing the flat clear.
// Far geometry fogs toward `skyHorizonColor`, so the horizon reads seamlessly.
//
// KE-0406 adds the **sun** to it: a small disc with a soft halo, placed at
// `-light.direction` — the very vector `lit_linear` shades with, so turning the
// sun moves the disc and the lighting together and they cannot drift apart. It
// stays a single fullscreen pass with no new geometry: the per-pixel view ray is
// unprojected from the NDC the vertex stage already interpolates, and "is this
// pixel the sun?" is one dot product plus two smoothsteps.

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

/// World-space view ray through a fullscreen-pass pixel, from its NDC.
///
/// The sky owns no geometry, so it recovers the ray by unprojecting the pixel at
/// both ends of the depth range through the camera's inverse view-projection and
/// taking the direction between them. That works for any projection the seam
/// pushes, and degenerates safely: with an identity view-projection (the backend's
/// state before the first camera push) it yields a constant `(0, 0, 1)` rather
/// than NaNs.
inline float3 sky_view_ray(float2 ndc, constant Light& light) {
    float4 nearH = light.inverseViewProjection * float4(ndc, 0.0, 1.0);
    float4 farH  = light.inverseViewProjection * float4(ndc, 1.0, 1.0);
    return normalize(farH.xyz / farH.w - nearH.xyz / nearH.w);
}

/// Vertical gradient sky **plus the sun disc and its glow** (KE-0406), tonemapped
/// + sRGB-encoded like the rest of the scene — so a blazing disc clips to white
/// through the same ACES curve as everything else instead of blowing out.
fragment float4 sky_fragment_main(SkyOut in [[stage_in]],
                                  constant Light& light [[buffer(0)]]) {
    float t = clamp(in.ndc.y * 0.5 + 0.5, 0.0, 1.0);
    // Smooth the horizon->zenith transition a touch.
    t = t * t * (3.0 - 2.0 * t);
    float3 sky = mix(light.skyHorizonColor, light.skyTopColor, t);

    // The sun: one dot product between this pixel's view ray and the direction the
    // sunlight arrives *from*. `light.direction` is the direction it travels, so
    // the sun is at its negation — the same single source of truth the shading
    // uses, which is what keeps the disc and the highlights on the same sun.
    float3 rayDir = sky_view_ray(in.ndc, light);
    float cosToSun = dot(rayDir, normalize(-light.direction));

    // Core disc, antialiased over the rim by the smoothstep, and a halo that falls
    // off into the gradient (cubed so it stays tight near the disc and vanishes
    // gently). Both are exactly 0 outside their extents, so a frame whose sun is
    // off-screen is bit-identical to the plain gradient.
    float disc = smoothstep(SUN_DISC_COS_OUTER, SUN_DISC_COS_INNER, cosToSun);
    float glow = smoothstep(SUN_GLOW_COS, 1.0, cosToSun);
    glow = glow * glow * glow;

    // Scaled by the sun's own intensity so a dimmer sun also has a dimmer disc.
    float3 sun = light.color * light.diffuseIntensity
               * (disc * SUN_DISC_GAIN + glow * SUN_GLOW_GAIN);

    return float4(present_color(sky + sun), 1.0);
}

// ============================================================================
// Textured pipeline (KE-0403)
// ============================================================================
//
// A second pipeline reading the `[pos,normal,uv]` layout. It samples the bound
// base-color texture at the interpolated UV (trilinear, mipmapped) and runs the
// same lighting + fog + shadow-map + present math as the untextured path.

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
                                       sampler baseColorSampler [[sampler(0)]],
                                       depth2d<float> shadowMap [[texture(1)]]) {
    float4 sampled = baseColor.sample(baseColorSampler, in.uv);
    float3 albedo = sampled.rgb * material.baseColorFactor.rgb;

    float shadow = shadow_visibility(in.worldPosition, in.viewDepth, light, shadowMap);
    float3 lit = lit_linear(albedo, in.worldNormal, in.worldPosition,
                            in.viewDepth, shadow, light);
    return float4(present_color(lit), sampled.a * material.baseColorFactor.a);
}

// ============================================================================
// 2D overlay / HUD pass (KE-0404)
// ============================================================================
//
// Screen-space quads drawn AFTER the 3D scene: orthographic, no depth test or
// write, alpha blended. Positions arrive in pixels with the origin at the
// drawable's top-left; the vertex shader maps them to NDC using the viewport
// size. `mode` selects how the texel is used (see OverlayFill on the Rust side):
//   0 = solid   (ignore the texture, use the vertex color)
//   1 = textured(sample RGBA, multiply by the vertex color)
//   2 = SDF     (distance in .r -> crisp coverage, tinted by the vertex color)

/// Per-vertex overlay data: pixel position, atlas UV, RGBA tint, fill mode.
struct OverlayVertexIn {
    float2 position [[attribute(0)]];
    float2 uv       [[attribute(1)]];
    float4 color    [[attribute(2)]];
    float  mode     [[attribute(3)]];
};

struct OverlayVertexOut {
    float4 position [[position]];
    float2 uv;
    float4 color;
    float  mode;
};

/// Per-pass overlay uniforms: the drawable size used for the ortho mapping.
struct OverlayUniforms {
    float2 viewportSize;               // Drawable size in pixels
};

/// Overlay vertex shader: pixels (origin top-left, +Y down) -> NDC.
vertex OverlayVertexOut overlay_vertex_main(OverlayVertexIn in [[stage_in]],
                                            constant OverlayUniforms& uniforms [[buffer(1)]]) {
    OverlayVertexOut out;
    // Pixel -> [0,1] -> NDC, flipping Y because NDC is +Y up.
    float2 unit = in.position / uniforms.viewportSize;
    out.position = float4(unit.x * 2.0 - 1.0, 1.0 - unit.y * 2.0, 0.0, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    out.mode = in.mode;
    return out;
}

/// Overlay fragment shader. Output is already display-referred (the overlay is
/// authored in display space and composites over the tonemapped scene), so it
/// does NOT run the scene's tonemap/encode path.
fragment float4 overlay_fragment_main(OverlayVertexOut in [[stage_in]],
                                      texture2d<float> atlas [[texture(0)]],
                                      sampler atlasSampler [[sampler(0)]]) {
    if (in.mode < 0.5) {
        // Solid.
        return in.color;
    }

    float4 sampled = atlas.sample(atlasSampler, in.uv);

    if (in.mode < 1.5) {
        // Straight textured.
        return sampled * in.color;
    }

    // SDF text: the atlas stores signed distance in .r, 0.5 being the edge.
    // Derive a screen-space-consistent antialiasing width from the distance
    // field's own gradient so glyphs stay crisp at any scale.
    float dist = sampled.r;
    float width = max(fwidth(dist), 1e-4);
    float coverage = smoothstep(0.5 - width, 0.5 + width, dist);
    return float4(in.color.rgb, in.color.a * coverage);
}
