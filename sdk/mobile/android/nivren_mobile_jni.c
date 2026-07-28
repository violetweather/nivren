#include <jni.h>
#include <stdint.h>
#include <stdlib.h>

#include "nivren.h"

#define NIVREN_MOBILE_MAXIMUM (16u * 1024u * 1024u)

static void throw_failure(JNIEnv *environment, const char *name, const char *message) {
    jclass failure = (*environment)->FindClass(environment, name);
    if (failure != NULL) {
        (*environment)->ThrowNew(environment, failure, message);
    }
}

JNIEXPORT jint JNICALL Java_org_nivren_NivrenMobile_abiVersion(
    JNIEnv *environment,
    jobject receiver
) {
    (void)environment;
    (void)receiver;
    return (jint)nivren_abi_version();
}

JNIEXPORT jbyteArray JNICALL Java_org_nivren_NivrenMobile_invoke(
    JNIEnv *environment,
    jobject receiver,
    jbyteArray source,
    jint operation
) {
    (void)receiver;
    if (source == NULL) {
        throw_failure(environment, "java/lang/IllegalArgumentException", "Nivren source is null");
        return NULL;
    }
    jsize length = (*environment)->GetArrayLength(environment, source);
    if (length < 0 || (uint32_t)length > NIVREN_MOBILE_MAXIMUM) {
        throw_failure(environment, "java/lang/IllegalArgumentException", "Nivren input exceeds 16 MiB");
        return NULL;
    }
    uint8_t *input = length == 0 ? NULL : malloc((size_t)length);
    if (length != 0 && input == NULL) {
        throw_failure(environment, "java/lang/OutOfMemoryError", "Cannot allocate Nivren input");
        return NULL;
    }
    if (length != 0) {
        (*environment)->GetByteArrayRegion(environment, source, 0, length, (jbyte *)input);
        if ((*environment)->ExceptionCheck(environment)) {
            free(input);
            return NULL;
        }
    }
    NivrenBuffer result;
    switch (operation) {
        case 0: result = nivren_check_utf8(input, (size_t)length); break;
        case 1: result = nivren_format_utf8(input, (size_t)length); break;
        case 2: result = nivren_run_utf8(input, (size_t)length); break;
        case 3: result = nivren_run_native_utf8(input, (size_t)length); break;
        default:
            free(input);
            throw_failure(environment, "java/lang/IllegalArgumentException", "Unknown Nivren mobile operation");
            return NULL;
    }
    free(input);
    if (result.length > NIVREN_MOBILE_MAXIMUM || (result.length != 0 && result.data == NULL)) {
        nivren_buffer_free(result);
        throw_failure(environment, "java/lang/IllegalStateException", "Nivren returned an invalid buffer");
        return NULL;
    }
    if (result.status != 0) {
        nivren_buffer_free(result);
        throw_failure(environment, "java/lang/IllegalStateException", "Nivren compilation or execution failed");
        return NULL;
    }
    jbyteArray output = (*environment)->NewByteArray(environment, (jsize)result.length);
    if (output != NULL && result.length != 0) {
        (*environment)->SetByteArrayRegion(
            environment,
            output,
            0,
            (jsize)result.length,
            (const jbyte *)result.data
        );
    }
    nivren_buffer_free(result);
    return output;
}
