// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.
//
// GPU Ray Tracer - Metal Compute Shader
//
// This shader implements iterative path tracing to avoid Metal's buggy recursion.
// It supports up to 3 bounces with per-triangle material properties.
//
// Key Features:
// - Möller-Trumbore ray-triangle intersection
// - AABB bounding boxes for 2-3x speedup
// - Shadow rays with occlusion testing
// - Per-triangle reflectivity (0.0=matte, 1.0=mirror)
// - Pre-calculated camera basis vectors
//
// Performance Optimizations:
// - Early AABB rejection before expensive intersection tests
// - Shadow ray early termination on first hit
// - Shared camera calculation across all threads
// - Single consolidated EPSILON constant
//
// Known Issues:
// - Metal shader recursion corrupts local variables at depth 2-3
// - Workaround: Iterative loop with manual state tracking
//

#include <metal_stdlib>
using namespace metal;

// Global constants
constant float EPSILON = 0.000001;

// Axis-Aligned Bounding Box for early ray rejection
struct AABB {
    packed_float3 min;  // Minimum corner
    packed_float3 max;  // Maximum corner
};

// Triangle geometry with material properties and acceleration structure
// Total size: 124 bytes (aligned for Metal buffer)
struct Triangle {
    // Geometry (84 bytes)
    packed_float3 p0;           // Vertex 0
    packed_float3 p1;           // Vertex 1
    packed_float3 p2;           // Vertex 2
    packed_float3 n0;           // Normal at vertex 0
    packed_float3 n1;           // Normal at vertex 1
    packed_float3 n2;           // Normal at vertex 2
    packed_float3 color;        // Base color (RGB)

    // Acceleration (24 bytes)
    AABB bounds;                // Bounding box for early rejection

    // Material (16 bytes)
    float reflectivity;         // 0.0 = completely matte, 1.0 = perfect mirror
    float _padding1;            // Alignment padding
    float _padding2;
    float _padding3;
};

// Ray tracing parameters passed from CPU
struct RayTracingParams {
    packed_float3 camera_position;
    float aspect_ratio;
    packed_float3 camera_target;
    int max_depth;                      // Maximum reflection bounces
    packed_float3 background_color;
    float default_reflectivity;         // Legacy: now per-triangle

    // Pre-calculated camera basis vectors (optimization)
    // Calculated once on CPU, shared by all GPU threads
    packed_float3 camera_forward;
    float _padding1;
    packed_float3 camera_right;
    float _padding2;
    packed_float3 camera_up;
    float _padding3;
};

// Phong lighting uniforms
struct LightUniforms {
    packed_float3 direction;        // Light direction (normalized)
    float _padding1;
    packed_float3 color;            // Light color (RGB)
    float _padding2;
    float ambient_intensity;        // Ambient light strength
    float diffuse_intensity;        // Diffuse light strength
    float specular_intensity;       // Specular highlight strength
    float shininess;                // Specular exponent
};

// Fast AABB-ray intersection test for early rejection
bool ray_aabb_intersection(float3 ray_origin, float3 ray_dir, AABB bounds) {
    float3 inv_dir = 1.0 / ray_dir;
    float3 t_min = (bounds.min - ray_origin) * inv_dir;
    float3 t_max = (bounds.max - ray_origin) * inv_dir;

    float3 t1 = min(t_min, t_max);
    float3 t2 = max(t_min, t_max);

    float t_near = max(max(t1.x, t1.y), t1.z);
    float t_far = min(min(t2.x, t2.y), t2.z);

    return t_near <= t_far && t_far > EPSILON;
}

// Möller–Trumbore ray-triangle intersection
bool ray_triangle_intersection(
    float3 ray_origin,
    float3 ray_dir,
    Triangle tri,
    thread float& t,
    thread float& u,
    thread float& v
) {
    float3 edge1 = tri.p1 - tri.p0;
    float3 edge2 = tri.p2 - tri.p0;

    float3 h = cross(ray_dir, edge2);
    float a = dot(edge1, h);

    if (abs(a) < EPSILON) {
        return false; // Ray is parallel
    }

    float f = 1.0 / a;
    float3 s = ray_origin - tri.p0;
    u = f * dot(s, h);

    if (u < 0.0 || u > 1.0) {
        return false;
    }

    float3 q = cross(s, edge1);
    v = f * dot(ray_dir, q);

    if (v < 0.0 || u + v > 1.0) {
        return false;
    }

    t = f * dot(edge2, q);

    return t > EPSILON;
}

// Check if a ray is occluded by geometry (for shadow rays)
bool is_occluded(
    float3 ray_origin,
    float3 ray_dir,
    float max_distance,
    constant Triangle* triangles,
    uint triangle_count
) {
    for (uint i = 0; i < triangle_count; i++) {
        if (!ray_aabb_intersection(ray_origin, ray_dir, triangles[i].bounds)) {
            continue;
        }

        float t, u, v;
        if (ray_triangle_intersection(ray_origin, ray_dir, triangles[i], t, u, v)) {
            if (t < max_distance) {
                return true; // Found an occluder
            }
        }
    }
    return false; // No occlusion
}

// Iterative ray tracing to avoid Metal recursion bugs
float3 trace_ray(
    float3 ray_origin,
    float3 ray_dir,
    constant Triangle* triangles,
    uint triangle_count,
    constant LightUniforms& light,
    constant RayTracingParams& params
) {
    const int MAX_BOUNCES = 10; // Hard limit to prevent infinite loops

    float3 final_color = float3(0.0);
    float3 accumulated_reflectivity = float3(1.0);

    float3 current_origin = ray_origin;
    float3 current_dir = ray_dir;

    int effective_max_depth = min(params.max_depth, MAX_BOUNCES);

    for (int bounce = 0; bounce < effective_max_depth; bounce++) {
        // Find closest intersection
        float closest_t = INFINITY;
        int closest_idx = -1;
        float closest_u = 0.0;
        float closest_v = 0.0;

        for (uint i = 0; i < triangle_count; i++) {
            if (!ray_aabb_intersection(current_origin, current_dir, triangles[i].bounds)) {
                continue;
            }

            float t, tri_u, tri_v;
            if (ray_triangle_intersection(current_origin, current_dir, triangles[i], t, tri_u, tri_v)) {
                if (t < closest_t) {
                    closest_t = t;
                    closest_idx = i;
                    closest_u = tri_u;
                    closest_v = tri_v;
                }
            }
        }

        if (closest_idx < 0) {
            // No intersection - add background contribution
            final_color += accumulated_reflectivity * params.background_color;
            break;
        }

        Triangle tri = triangles[closest_idx];
        float w = 1.0 - closest_u - closest_v;

        // Interpolate normal
        float3 normal = normalize(w * tri.n0 + closest_u * tri.n1 + closest_v * tri.n2);

        // Calculate hit point
        float3 hit_point = current_origin + current_dir * closest_t;

        // Phong lighting
        float3 light_dir_normalized = normalize(light.direction);
        float ndotl = max(dot(normal, -light_dir_normalized), 0.0);

        // Check for shadows by casting a ray toward the light
        bool in_shadow = false;
        if (ndotl > 0.0) {
            float3 shadow_ray_origin = hit_point + normal * EPSILON;
            float3 shadow_ray_dir = -light_dir_normalized;
            in_shadow = is_occluded(shadow_ray_origin, shadow_ray_dir, INFINITY, triangles, triangle_count);
        }

        // Ambient component (always present)
        float3 ambient = light.ambient_intensity * light.color;

        // Diffuse and specular only if not in shadow
        float3 diffuse = float3(0.0);
        float3 specular = float3(0.0);

        if (!in_shadow && ndotl > 0.0) {
            diffuse = light.diffuse_intensity * ndotl * light.color;

            float3 view_dir = -current_dir;
            float3 half_dir = normalize(-light_dir_normalized + view_dir);
            float spec = pow(max(dot(normal, half_dir), 0.0), light.shininess);
            specular = light.specular_intensity * spec * light.color;
        }

        // Combine lighting components
        float3 lighting = ambient + diffuse + specular;
        float3 local_color = tri.color * lighting;

        // Use per-triangle reflectivity
        float reflectivity = tri.reflectivity;

        // Add this bounce's contribution
        final_color += accumulated_reflectivity * local_color * (1.0 - reflectivity);

        // Check if we should continue bouncing
        if (bounce >= effective_max_depth - 1) {
            break;
        }

        // Update accumulated reflectivity for next bounce
        accumulated_reflectivity *= reflectivity;

        // Set up reflection ray for next iteration
        current_dir = reflect(current_dir, normal);
        current_origin = hit_point + normal * EPSILON;
    }

    return final_color;
}

kernel void raytrace_kernel(
    texture2d<float, access::write> output_texture [[texture(0)]],
    constant Triangle* triangles [[buffer(0)]],
    constant RayTracingParams& params [[buffer(1)]],
    constant LightUniforms& light [[buffer(2)]],
    constant uint& triangle_count [[buffer(3)]],
    uint2 gid [[thread_position_in_grid]]
) {
    uint width = output_texture.get_width();
    uint height = output_texture.get_height();

    if (gid.x >= width || gid.y >= height) {
        return;
    }

    // Use pre-calculated camera basis vectors
    float3 forward = float3(params.camera_forward);
    float3 right = float3(params.camera_right);
    float3 up = float3(params.camera_up);

    // NDC coordinates (-1 to 1)
    float aspect = float(width) / float(height);
    float2 ndc = float2(
        ((float(gid.x) + 0.5) / float(width) - 0.5) * 2.0,
        (0.5 - (float(gid.y) + 0.5) / float(height)) * 2.0
    );

    float fov = 60.0 * 3.14159265 / 180.0;
    float half_height = tan(fov / 2.0);

    // Calculate ray direction
    float3 ray_dir = normalize(
        forward +
        right * ndc.x * half_height * aspect +
        up * ndc.y * half_height
    );

    // Trace the ray with reflections using the trace_ray function
    float3 color = trace_ray(params.camera_position, ray_dir, triangles, triangle_count, light, params);

    // Write to output texture (BGRA format)
    output_texture.write(float4(color, 1.0), gid);
}
