#ifndef KOHARU_VISION_NATIVE_H
#define KOHARU_VISION_NATIVE_H
#include <stdint.h>
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#endif
void *koharu_diffusion_open(const char *directory, int flux, char *error, size_t capacity);
int koharu_diffusion_run(void *handle, const uint8_t *rgb, const uint8_t *mask, int edge, uint8_t *output, char *error, size_t capacity);
void koharu_diffusion_close(void *handle);
void *koharu_paddle_open(const char *directory, char *error, size_t capacity);
int koharu_paddle_read(void *handle, const uint8_t *rgb, int width, int height, char *output, size_t capacity);
void koharu_paddle_close(void *handle);
#ifdef __cplusplus
}
#endif
#endif
