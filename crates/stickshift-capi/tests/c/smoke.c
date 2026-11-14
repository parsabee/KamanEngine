/*
 * The C API smoke test: a C program built against stickshift.h and libstickshift.
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
#include <stdlib.h>
#include <string.h>

#include "stickshift.h"

#define CHECK(expr, expected)                                                         \
    do {                                                                              \
        StickShiftResult check_result_ = (expr);                                           \
        if (check_result_ != (expected)) {                                            \
            fprintf(stderr, "%s:%d: %s returned %s, expected %s (%s)\n", __FILE__,    \
                    __LINE__, #expr, stickshift_result_name(check_result_), #expected,     \
                    stickshift_last_error_message());                                      \
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
static StickShiftResult model_result = STICKSHIFT_OK;
static void on_model(void *user, const StickShiftEvent *event) {
    (void)user;
    model_events++;
    model_result = event->kind == STICKSHIFT_EVENT_RESOURCE_READY ? STICKSHIFT_OK : event->result;
}

static StickShiftData copied(const void *ptr, size_t len) {
    StickShiftData data = {ptr, len, NULL, NULL};
    return data;
}

int main(int argc, char **argv) {
    int use_null = argc > 1 && strcmp(argv[1], "--null") == 0;
    const char *model_path = argc > 1 + use_null ? argv[1 + use_null] : NULL;
    REQUIRE(model_path != NULL);
    REQUIRE(stickshift_api_version() == STICKSHIFT_API_VERSION);

    StickShiftContextDesc desc = {STICKSHIFT_API_VERSION, use_null ? STICKSHIFT_BACKEND_NULL : STICKSHIFT_BACKEND_METAL,
                             NULL, WIDTH, HEIGHT, 1.0f};
    StickShiftContext *ctx = NULL;
    StickShiftResult created = stickshift_context_create(&desc, &ctx);
    if (!use_null && created == STICKSHIFT_ERROR_NO_DEVICE) {
        /* CI sets STICKSHIFT_REQUIRE_GPU=1: a runner that lost its GPU must fail, not skip. */
        const char *require = getenv("STICKSHIFT_REQUIRE_GPU");
        if (require != NULL && (strcmp(require, "1") == 0 || strcmp(require, "true") == 0)) {
            fprintf(stderr, "c smoke: no Metal device, and STICKSHIFT_REQUIRE_GPU is set (%s)\n",
                    stickshift_last_error_message());
            return 1;
        }
        printf("c smoke: skipped (no Metal device: %s)\n", stickshift_last_error_message());
        return 0;
    }
    CHECK(created, STICKSHIFT_OK);

    StickShiftEvent events[16];
    uint32_t count = 0;
    CHECK(stickshift_poll_events(ctx, events, 16, &count), STICKSHIFT_OK);
    REQUIRE(count == 1 && events[0].kind == STICKSHIFT_EVENT_CONTEXT_READY);

    /* A red triangle, copied. */
    float triangle[3][9] = {
        {-0.6f, -0.5f, 0.0f, 0, 0, 1, 0.9f, 0.1f, 0.1f},
        {0.6f, -0.5f, 0.0f, 0, 0, 1, 0.9f, 0.1f, 0.1f},
        {0.0f, 0.6f, 0.0f, 0, 0, 1, 0.9f, 0.1f, 0.1f},
    };
    uint32_t indices[3] = {0, 1, 2};
    StickShiftMeshDesc mesh_desc = {STICKSHIFT_VERTEX_LAYOUT_POS_NORMAL_COLOR,
                               copied(triangle, sizeof triangle), copied(indices, sizeof indices)};
    StickShiftMesh mesh = {0};
    CHECK(stickshift_mesh_create(ctx, &mesh_desc, NULL, &mesh), STICKSHIFT_OK);
    REQUIRE(stickshift_resource_state(ctx, mesh.id) == STICKSHIFT_RESOURCE_STATE_READY);

    /* A 2x2 texture whose pixels are lent, not copied. */
    static const uint8_t white[16] = {255, 255, 255, 255, 255, 255, 255, 255,
                                      255, 255, 255, 255, 255, 255, 255, 255};
    StickShiftTextureDesc texture_desc = {2, 2, {white, sizeof white, count_release, NULL},
                                     STICKSHIFT_COLOR_SPACE_SRGB, 0};
    StickShiftTexture texture = {0};
    CHECK(stickshift_texture_create(ctx, &texture_desc, NULL, &texture), STICKSHIFT_OK);
    REQUIRE(releases == 1);

    /* A model, parsed off-thread; its event goes to the callback. */
    StickShiftCompletion completion = {on_model, NULL};
    StickShiftModel model = {0};
    CHECK(stickshift_model_load_file(ctx, model_path, &completion, &model), STICKSHIFT_OK);
    CHECK(stickshift_context_flush(ctx), STICKSHIFT_OK);
    REQUIRE(model_events == 1);
    CHECK(model_result, STICKSHIFT_OK);
    StickShiftVec3 lo, hi;
    CHECK(stickshift_model_get_bounds(ctx, model, &lo, &hi), STICKSHIFT_OK);
    REQUIRE(hi.x > lo.x);

    /* One frame: the triangle, the model, a HUD bar, and a read-back. */
    StickShiftFrame *frame = NULL;
    StickShiftFrame *second = NULL;
    CHECK(stickshift_frame_acquire(ctx, &frame), STICKSHIFT_OK);
    CHECK(stickshift_frame_acquire(ctx, &second), STICKSHIFT_ERROR_WRONG_STATE);
    REQUIRE(second == NULL);

    StickShiftVec3 eye = {0.0f, 0.5f, 3.0f}, target = {0.0f, 0.0f, 0.0f}, up = {0.0f, 1.0f, 0.0f};
    StickShiftCamera camera;
    camera.view_projection =
        stickshift_mat4_mul(stickshift_mat4_perspective(0.9f, (float)WIDTH / HEIGHT, 0.1f, 50.0f),
                       stickshift_mat4_look_at(eye, target, up));
    camera.position = eye;
    CHECK(stickshift_frame_set_camera(frame, &camera), STICKSHIFT_OK);

    StickShiftMat4 identity = stickshift_mat4_identity();
    CHECK(stickshift_frame_draw_mesh(frame, mesh, &identity, NULL), STICKSHIFT_OK);
    StickShiftVec3 beside = {1.5f, 0.0f, -1.0f}, scale = {0.5f, 0.5f, 0.5f};
    StickShiftQuat spin = stickshift_quat_from_axis_angle(up, 0.6f);
    StickShiftMat4 world = stickshift_mat4_from_trs(beside, spin, scale);
    CHECK(stickshift_frame_draw_model(frame, model, &world, NULL), STICKSHIFT_OK);
    StickShiftQuad bar = {2, 2, 20, 4, 0, 0, 1, 1, {1, 1, 1, 0.8f}, STICKSHIFT_FILL_SOLID, {0}};
    CHECK(stickshift_frame_draw_quad(frame, &bar), STICKSHIFT_OK);

    static uint8_t pixels[WIDTH * HEIGHT * 4];
    memset(pixels, 0xAB, sizeof pixels);
    CHECK(stickshift_frame_read_pixels_async(frame, pixels, sizeof pixels, NULL), STICKSHIFT_OK);
    uint64_t number = 0;
    CHECK(stickshift_frame_submit(frame, NULL, &number), STICKSHIFT_OK);
    REQUIRE(number == 1);

    CHECK(stickshift_context_flush(ctx), STICKSHIFT_OK);
    int completed = 0, read_back = 0;
    do {
        CHECK(stickshift_poll_events(ctx, events, 16, &count), STICKSHIFT_OK);
        for (uint32_t i = 0; i < count; i++) {
            completed |= events[i].kind == STICKSHIFT_EVENT_FRAME_COMPLETED && events[i].frame == 1;
            read_back |= events[i].kind == STICKSHIFT_EVENT_READBACK_COMPLETE && events[i].frame == 1;
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

    StickShiftStats stats;
    CHECK(stickshift_context_get_stats(ctx, &stats), STICKSHIFT_OK);
    REQUIRE(stats.frames_submitted == 1 && stats.frames_completed == 1);
    REQUIRE(stats.draws_submitted >= 2 && stats.draws_skipped == 0);
    REQUIRE(stats.meshes == 1 && stats.textures == 1 && stats.models == 1);

    CHECK(stickshift_model_destroy(ctx, model), STICKSHIFT_OK);
    CHECK(stickshift_model_destroy(ctx, model), STICKSHIFT_ERROR_INVALID_HANDLE);
    REQUIRE(strlen(stickshift_last_error_message()) > 0);
    CHECK(stickshift_context_destroy(ctx), STICKSHIFT_OK);

    printf("c smoke: OK (%s backend, frame %llu)\n", use_null ? "null" : "Metal",
           (unsigned long long)number);
    return 0;
}
