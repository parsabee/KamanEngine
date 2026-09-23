// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.
//
// Rasterization Shader - Phong Lighting Model
//
// This shader implements traditional GPU rasterization with Blinn-Phong lighting.
// Used for standard mesh rendering with position, normal, and color vertex attributes.

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
    float3 worldPosition;              // World-space position for lighting
    float3 color;                      // Interpolated vertex color
};

// ============================================================================
// Uniform Structures
// ============================================================================

/// Transformation matrices passed from CPU per draw call
struct Uniforms {
    float4x4 modelViewProjection;      // Combined model-view-projection matrix
};

/// Lighting parameters (directional light with Phong components)
struct Light {
    float3 direction;                  // Light direction in world space
    float3 color;                      // Light color (RGB)
    float ambientIntensity;            // Ambient light strength (constant illumination)
    float diffuseIntensity;            // Diffuse reflection strength (angle-dependent)
    float specularIntensity;           // Specular highlight strength (view-dependent)
    float shininess;                   // Phong exponent (controls highlight tightness)
};

// ============================================================================
// Vertex Shader
// ============================================================================

/// Transform vertices to clip space and pass data to fragment shader
vertex VertexOut vertex_main(VertexIn in [[stage_in]],
                             constant Uniforms& uniforms [[buffer(1)]]) {
    VertexOut out;

    // Transform position to clip space for rasterization
    out.position = uniforms.modelViewProjection * float4(in.position, 1.0);

    // Pass through attributes for fragment shader (will be interpolated)
    out.worldNormal = in.normal;
    out.worldPosition = in.position;
    out.color = in.color;

    return out;
}

// ============================================================================
// Fragment Shader
// ============================================================================

/// Calculate per-pixel lighting using Blinn-Phong shading model
fragment float4 fragment_main(VertexOut in [[stage_in]],
                              constant Light& light [[buffer(0)]]) {
    // Normalize the interpolated normal (interpolation can denormalize vectors)
    float3 normal = normalize(in.worldNormal);

    // Light direction points from surface to light source
    float3 lightDir = normalize(-light.direction);

    // ---- Ambient Component ----
    // Constant base illumination independent of geometry
    float3 ambient = light.ambientIntensity * light.color;

    // ---- Diffuse Component (Lambertian Reflection) ----
    // Intensity depends on angle between surface normal and light direction
    // Uses N·L where N is normal and L is light direction
    float diff = max(dot(normal, lightDir), 0.0);
    float3 diffuse = light.diffuseIntensity * diff * light.color;

    // ---- Specular Component (Blinn-Phong) ----
    // Highlights based on halfway vector between view and light directions
    float3 viewDir = normalize(float3(0.0, 0.0, 1.0));
    float3 halfDir = normalize(lightDir + viewDir);
    float spec = pow(max(dot(normal, halfDir), 0.0), light.shininess);
    float3 specular = light.specularIntensity * spec * light.color;

    // ---- Final Color ----
    // Combine all lighting components and modulate by surface color
    float3 result = (ambient + diffuse + specular) * in.color;

    return float4(result, 1.0);
}

// ============================================================================
// Textured pipeline (KE-0403)
// ============================================================================
//
// A second pipeline reading the `[pos,normal,uv]` layout. It samples the bound
// base-color texture at the interpolated UV (trilinear, mipmapped) and lights it
// with the same Blinn-Phong model, so an imported textured mesh renders with its
// texture instead of a vertex color. The untextured pipeline above is untouched,
// so the box scene's pixel-hash is unchanged.

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
    float2 uv;
};

/// Per-draw material parameters for the textured pipeline (base-color factor).
/// Mirrors the Rust `MaterialUniforms` (`[[buffer(2)]]`), 16 bytes.
struct MaterialUniforms {
    float4 baseColorFactor;            // Linear RGBA, multiplied with the texture
};

/// Textured vertex shader: transform to clip space, pass normal + UV through.
vertex TexturedVertexOut textured_vertex_main(TexturedVertexIn in [[stage_in]],
                                              constant Uniforms& uniforms [[buffer(1)]]) {
    TexturedVertexOut out;
    out.position = uniforms.modelViewProjection * float4(in.position, 1.0);
    out.worldNormal = in.normal;
    out.uv = in.uv;
    return out;
}

/// Textured fragment shader: sample the bound base-color texture (trilinear) at
/// the UV, modulate by the material base-color factor, and light with Blinn-Phong.
fragment float4 textured_fragment_main(TexturedVertexOut in [[stage_in]],
                                       constant Light& light [[buffer(0)]],
                                       constant MaterialUniforms& material [[buffer(2)]],
                                       texture2d<float> baseColor [[texture(0)]],
                                       sampler baseColorSampler [[sampler(0)]]) {
    float4 sampled = baseColor.sample(baseColorSampler, in.uv);
    float3 albedo = sampled.rgb * material.baseColorFactor.rgb;

    float3 normal = normalize(in.worldNormal);
    float3 lightDir = normalize(-light.direction);

    float3 ambient = light.ambientIntensity * light.color;
    float diff = max(dot(normal, lightDir), 0.0);
    float3 diffuse = light.diffuseIntensity * diff * light.color;

    float3 viewDir = normalize(float3(0.0, 0.0, 1.0));
    float3 halfDir = normalize(lightDir + viewDir);
    float spec = pow(max(dot(normal, halfDir), 0.0), light.shininess);
    float3 specular = light.specularIntensity * spec * light.color;

    float3 result = (ambient + diffuse + specular) * albedo;
    return float4(result, sampled.a * material.baseColorFactor.a);
}
