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

typedef NivrenBuffer (*NivrenHostCallback)(
    const uint8_t *name,
    size_t name_length,
    const uint8_t *request,
    size_t request_length,
    void *context
);
typedef void (*NivrenHostFree)(NivrenBuffer buffer, void *context);
typedef void (*NivrenAsyncComplete)(NivrenBuffer buffer, void *context);
typedef void (*NivrenWake)(void *context);
typedef struct NivrenAsyncRun NivrenAsyncRun;

uint32_t nivren_abi_version(void);
NivrenBuffer nivren_check_utf8(const uint8_t *source, size_t length);
NivrenBuffer nivren_format_utf8(const uint8_t *source, size_t length);
NivrenBuffer nivren_compile_utf8(const uint8_t *source, size_t length);

/*
 * Every run entry point below executes the program with only the default
 * capabilities: Task, Channel, Time, Log, and Random. Files, the network,
 * processes, the environment, and native code loading are denied unless the
 * host grants them through nivren_run_utf8_with_capabilities. The host
 * callback entry points additionally grant Native for host handles of every
 * kind, but never std.native.open.
 */
NivrenBuffer nivren_run_utf8(const uint8_t *source, size_t length);
NivrenBuffer nivren_run_native_utf8(const uint8_t *source, size_t length);
NivrenBuffer nivren_run_host_utf8(
    const uint8_t *source,
    size_t length,
    NivrenHostCallback callback,
    NivrenHostFree free_callback,
    void *context
);

/*
 * capabilities is a UTF-8 list separated by commas or newlines. Each entry is
 * a capability name, optionally scoped with "=": for example
 * "FileRead=path:./data,Network=host:api.example.com,Native=kind:database".
 * "*" grants every capability without scopes and must only be used for
 * fully trusted source. Pass both callbacks or neither. Callbacks may be
 * invoked concurrently from program worker threads and must be thread-safe.
 */
NivrenBuffer nivren_run_utf8_with_capabilities(
    const uint8_t *source,
    size_t length,
    const uint8_t *capabilities,
    size_t capabilities_length,
    NivrenHostCallback callback,
    NivrenHostFree free_callback,
    void *context
);
NivrenAsyncRun *nivren_run_async_utf8(
    const uint8_t *source,
    size_t length,
    NivrenAsyncComplete complete,
    NivrenWake wake,
    void *context
);
void nivren_async_run_cancel(NivrenAsyncRun *run);
uint32_t nivren_async_run_finished(const NivrenAsyncRun *run);
void nivren_async_run_free(NivrenAsyncRun *run);
void nivren_buffer_free(NivrenBuffer buffer);

#ifdef __cplusplus
}
#endif

#endif
