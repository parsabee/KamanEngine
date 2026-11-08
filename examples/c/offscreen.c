/*
 * A minimal KamanEngine C program: render a glTF model offscreen and save the
 * frame as a PPM image.
 *
 *   offscreen <model.gltf|model.glb> <out.ppm> [--null]
 *
 * Build it with scripts/test-c-api.sh, or by hand against the installed
 * library, e.g. with pkg-config:
 *
 *   cc -std=c11 offscreen.c $(pkg-config --cflags --libs kaman) -o offscreen
 *
 * Copyright (c) 2026 Parsa Bagheri. Released under the Apache-2.0 License.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "kaman.h"

enum { WIDTH = 320, HEIGHT = 240 };

static int fail(const char *what, KamanResult result) {
    fprintf(stderr, "%s failed: %s (%s)\n", what, kaman_result_name(result),
            kaman_last_error_message());
    return 1;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: %s <model.gltf|model.glb> <out.ppm> [--null]\n", argv[0]);
        return 2;
    }
    int use_null = argc > 3 && strcmp(argv[3], "--null") == 0;

    KamanContextDesc desc = {KAMAN_API_VERSION, use_null ? KAMAN_BACKEND_NULL : KAMAN_BACKEND_METAL,
                             NULL /* offscreen */, WIDTH, HEIGHT, 1.0f};
    KamanContext *ctx;
    KamanResult r = kaman_context_create(&desc, &ctx);
    if (r != KAMAN_OK) return fail("kaman_context_create", r);
    printf("device: %s\n", kaman_context_device_name(ctx));

    /* Loading returns at once; the parse runs on a worker thread. A real app
     * would keep rendering and watch for the RESOURCE_READY event; a tool can
     * simply wait. */
    KamanModel model;
    if ((r = kaman_model_load_file(ctx, argv[1], NULL, &model)) != KAMAN_OK)
        return fail("kaman_model_load_file", r);
    if ((r = kaman_context_flush(ctx)) != KAMAN_OK) return fail("kaman_context_flush", r);
    if (kaman_resource_state(ctx, model.id) != KAMAN_RESOURCE_STATE_READY) {
        KamanEvent events[8];
        uint32_t n = 0;
        kaman_poll_events(ctx, events, 8, &n);
        for (uint32_t i = 0; i < n; i++)
            if (events[i].kind == KAMAN_EVENT_RESOURCE_FAILED && events[i].message)
                fprintf(stderr, "load failed: %s\n", events[i].message);
        return 1;
    }

    /* Frame the model: put the camera back along +Z by twice its size. */
    KamanVec3 lo, hi;
    kaman_model_get_bounds(ctx, model, &lo, &hi);
    KamanVec3 center = {(lo.x + hi.x) / 2, (lo.y + hi.y) / 2, (lo.z + hi.z) / 2};
    float size = hi.x - lo.x > hi.y - lo.y ? hi.x - lo.x : hi.y - lo.y;
    KamanVec3 eye = {center.x + size, center.y + size * 0.6f, center.z + size * 1.8f};
    KamanVec3 up = {0, 1, 0};

    KamanFrame *frame;
    if ((r = kaman_frame_acquire(ctx, &frame)) != KAMAN_OK) return fail("kaman_frame_acquire", r);
    KamanCamera camera;
    camera.view_projection = kaman_mat4_mul(
        kaman_mat4_perspective(0.8f, (float)WIDTH / HEIGHT, size * 0.05f, size * 20.0f),
        kaman_mat4_look_at(eye, center, up));
    camera.position = eye;
    kaman_frame_set_camera(frame, &camera);
    KamanMat4 world = kaman_mat4_identity();
    kaman_frame_draw_model(frame, model, &world, NULL);

    uint8_t *bgra = malloc(WIDTH * HEIGHT * 4);
    if (!bgra) return 1;
    kaman_frame_read_pixels_async(frame, bgra, WIDTH * HEIGHT * 4, NULL);
    if ((r = kaman_frame_submit(frame, NULL, NULL)) != KAMAN_OK) return fail("kaman_frame_submit", r);
    if ((r = kaman_context_flush(ctx)) != KAMAN_OK) return fail("kaman_context_flush", r);

    FILE *out = fopen(argv[2], "wb");
    if (!out) {
        perror(argv[2]);
        return 1;
    }
    fprintf(out, "P6\n%d %d\n255\n", WIDTH, HEIGHT);
    for (size_t i = 0; i < (size_t)WIDTH * HEIGHT; i++) {
        uint8_t rgb[3] = {bgra[i * 4 + 2], bgra[i * 4 + 1], bgra[i * 4 + 0]};
        fwrite(rgb, 1, 3, out);
    }
    fclose(out);
    free(bgra);
    kaman_context_destroy(ctx);
    printf("wrote %s (%dx%d)\n", argv[2], WIDTH, HEIGHT);
    return 0;
}
