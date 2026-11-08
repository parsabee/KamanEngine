/*
 * The C API smoke test: a C program built against kaman.h and libkaman.
 *
 *   smoke [--null] <model.gltf>
 *
 * Exercises the whole lifecycle through the header (context, mesh, borrowed
 * texture data, a model loaded off-thread with a completion callback, a frame
 * with a read-back, events, stats) and checks the pixels: something was drawn
 * on Metal, zeros on the null backend. Exits 0 on success, 1 on failure, and 0
 * with a message when Metal is requested but no device exists.
 *
 * Run by scripts/test-c-api.sh (and CI).
 *
 * Copyright (c) 2026 Parsa Bagheri. Released under the Apache-2.0 License.
 */

#include <stdio.h>
#include <string.h>

#include "kaman.h"

#define CHECK(expr, expected)                                                         \
    do {                                                                              \
        KamanResult check_result_ = (expr);                                           \
        if (check_result_ != (expected)) {                                            \
            fprintf(stderr, "%s:%d: %s returned %s, expected %s (%s)\n", __FILE__,    \
                    __LINE__, #expr, kaman_result_name(check_result_), #expected,     \
                    kaman_last_error_message());                                      \
            return 1;                                                                 \
        }                                                                             \
    } while (0)

#define REQUIRE(cond)                                                                 \
    do {                                                                              \
        if (!(cond)) {                                                                \
            fprintf(stderr, "%s:%d: requirement failed: %s\n", __FILE__, __LINE__,    \
                    #cond);                                                           \
            return 1;                                                                 \
        }                                                                             \
    } while (0)

enum { WIDTH = 64, HEIGHT = 48 };

static int releases = 0;
static void count_release(void *user, const void *ptr, size_t len) {
    (void)user;
    (void)ptr;
    (void)len;
    releases++;
}

static int model_events = 0;
static KamanResult model_result = KAMAN_OK;
static void on_model(void *user, const KamanEvent *event) {
    (void)user;
    model_events++;
    model_result = event->kind == KAMAN_EVENT_RESOURCE_READY ? KAMAN_OK : event->result;
}

static KamanData copied(const void *ptr, size_t len) {
    KamanData data = {ptr, len, NULL, NULL};
    return data;
}

int main(int argc, char **argv) {
    int use_null = argc > 1 && strcmp(argv[1], "--null") == 0;
    const char *model_path = argc > 1 + use_null ? argv[1 + use_null] : NULL;
    REQUIRE(model_path != NULL);
    REQUIRE(kaman_api_version() == KAMAN_API_VERSION);

    KamanContextDesc desc = {KAMAN_API_VERSION, use_null ? KAMAN_BACKEND_NULL : KAMAN_BACKEND_METAL,
                             NULL, WIDTH, HEIGHT, 1.0f};
    KamanContext *ctx = NULL;
    KamanResult created = kaman_context_create(&desc, &ctx);
    if (!use_null && created == KAMAN_ERROR_NO_DEVICE) {
        printf("c smoke: skipped (no Metal device: %s)\n", kaman_last_error_message());
        return 0;
    }
    CHECK(created, KAMAN_OK);

    KamanEvent events[16];
    uint32_t count = 0;
    CHECK(kaman_poll_events(ctx, events, 16, &count), KAMAN_OK);
    REQUIRE(count == 1 && events[0].kind == KAMAN_EVENT_CONTEXT_READY);

    /* A red triangle, copied. */
    float triangle[3][9] = {
        {-0.6f, -0.5f, 0.0f, 0, 0, 1, 0.9f, 0.1f, 0.1f},
        {0.6f, -0.5f, 0.0f, 0, 0, 1, 0.9f, 0.1f, 0.1f},
        {0.0f, 0.6f, 0.0f, 0, 0, 1, 0.9f, 0.1f, 0.1f},
    };
    uint32_t indices[3] = {0, 1, 2};
    KamanMeshDesc mesh_desc = {KAMAN_VERTEX_LAYOUT_POS_NORMAL_COLOR,
                               copied(triangle, sizeof triangle), copied(indices, sizeof indices)};
    KamanMesh mesh = {0};
    CHECK(kaman_mesh_create(ctx, &mesh_desc, NULL, &mesh), KAMAN_OK);
    REQUIRE(kaman_resource_state(ctx, mesh.id) == KAMAN_RESOURCE_STATE_READY);

    /* A 2x2 texture whose pixels are lent, not copied. */
    static const uint8_t white[16] = {255, 255, 255, 255, 255, 255, 255, 255,
                                      255, 255, 255, 255, 255, 255, 255, 255};
    KamanTextureDesc texture_desc = {2, 2, {white, sizeof white, count_release, NULL}, 0};
    KamanTexture texture = {0};
    CHECK(kaman_texture_create(ctx, &texture_desc, NULL, &texture), KAMAN_OK);
    REQUIRE(releases == 1);

    /* A model, parsed off-thread; its event goes to the callback. */
    KamanCompletion completion = {on_model, NULL};
    KamanModel model = {0};
    CHECK(kaman_model_load_file(ctx, model_path, &completion, &model), KAMAN_OK);
    CHECK(kaman_context_flush(ctx), KAMAN_OK);
    REQUIRE(model_events == 1);
    CHECK(model_result, KAMAN_OK);
    KamanVec3 lo, hi;
    CHECK(kaman_model_get_bounds(ctx, model, &lo, &hi), KAMAN_OK);
    REQUIRE(hi.x > lo.x);

    /* One frame: the triangle, the model, a HUD bar, and a read-back. */
    KamanFrame *frame = NULL;
    KamanFrame *second = NULL;
    CHECK(kaman_frame_acquire(ctx, &frame), KAMAN_OK);
    CHECK(kaman_frame_acquire(ctx, &second), KAMAN_ERROR_WRONG_STATE);
    REQUIRE(second == NULL);

    KamanVec3 eye = {0.0f, 0.5f, 3.0f}, target = {0.0f, 0.0f, 0.0f}, up = {0.0f, 1.0f, 0.0f};
    KamanCamera camera;
    camera.view_projection =
        kaman_mat4_mul(kaman_mat4_perspective(0.9f, (float)WIDTH / HEIGHT, 0.1f, 50.0f),
                       kaman_mat4_look_at(eye, target, up));
    camera.position = eye;
    CHECK(kaman_frame_set_camera(frame, &camera), KAMAN_OK);

    KamanMat4 identity = kaman_mat4_identity();
    CHECK(kaman_frame_draw_mesh(frame, mesh, &identity, NULL), KAMAN_OK);
    KamanVec3 beside = {1.5f, 0.0f, -1.0f}, scale = {0.5f, 0.5f, 0.5f};
    KamanQuat spin = kaman_quat_from_axis_angle(up, 0.6f);
    KamanMat4 world = kaman_mat4_from_trs(beside, spin, scale);
    CHECK(kaman_frame_draw_model(frame, model, &world, NULL), KAMAN_OK);
    KamanQuad bar = {2, 2, 20, 4, 0, 0, 1, 1, {1, 1, 1, 0.8f}, KAMAN_FILL_SOLID, {0}};
    CHECK(kaman_frame_draw_quad(frame, &bar), KAMAN_OK);

    static uint8_t pixels[WIDTH * HEIGHT * 4];
    memset(pixels, 0xAB, sizeof pixels);
    CHECK(kaman_frame_read_pixels_async(frame, pixels, sizeof pixels, NULL), KAMAN_OK);
    uint64_t number = 0;
    CHECK(kaman_frame_submit(frame, NULL, &number), KAMAN_OK);
    REQUIRE(number == 1);

    CHECK(kaman_context_flush(ctx), KAMAN_OK);
    int completed = 0, read_back = 0;
    do {
        CHECK(kaman_poll_events(ctx, events, 16, &count), KAMAN_OK);
        for (uint32_t i = 0; i < count; i++) {
            completed |= events[i].kind == KAMAN_EVENT_FRAME_COMPLETED && events[i].frame == 1;
            read_back |= events[i].kind == KAMAN_EVENT_READBACK_COMPLETE && events[i].frame == 1;
        }
    } while (count == 16);
    REQUIRE(completed && read_back);

    size_t center = ((HEIGHT / 2) * WIDTH + WIDTH / 2) * 4, corner = (HEIGHT - 1) * WIDTH * 4;
    if (use_null) {
        for (size_t i = 0; i < sizeof pixels; i++) REQUIRE(pixels[i] == 0);
    } else {
        /* The triangle covers the centre; the bottom-left corner is sky. */
        REQUIRE(memcmp(pixels + center, pixels + corner, 4) != 0);
        REQUIRE(pixels[center + 2] > pixels[center + 1]); /* BGRA: red dominates */
    }

    KamanStats stats;
    CHECK(kaman_context_get_stats(ctx, &stats), KAMAN_OK);
    REQUIRE(stats.frames_submitted == 1 && stats.frames_completed == 1);
    REQUIRE(stats.draws_submitted >= 2 && stats.draws_skipped == 0);
    REQUIRE(stats.meshes == 1 && stats.textures == 1 && stats.models == 1);

    CHECK(kaman_model_destroy(ctx, model), KAMAN_OK);
    CHECK(kaman_model_destroy(ctx, model), KAMAN_ERROR_INVALID_HANDLE);
    REQUIRE(strlen(kaman_last_error_message()) > 0);
    CHECK(kaman_context_destroy(ctx), KAMAN_OK);

    printf("c smoke: OK (%s backend, frame %llu)\n", use_null ? "null" : "Metal",
           (unsigned long long)number);
    return 0;
}
