#ifndef NIVREN_H
#define NIVREN_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct NivrenBuffer {
    uint8_t *data;
    size_t length;
    size_t capacity;
    uint32_t status;
} NivrenBuffer;

NivrenBuffer nivren_run_utf8(const uint8_t *source, size_t length);
void nivren_buffer_free(NivrenBuffer buffer);

#ifdef __cplusplus
}
#endif

#endif
