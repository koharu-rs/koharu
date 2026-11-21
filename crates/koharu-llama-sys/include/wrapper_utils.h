// From utilityai/llama-cpp-rs at 0364f5d, llama-cpp-sys-2/wrapper_utils.h. Its includes are
// relative to a llama.cpp submodule there; here the build's include paths find them.
#pragma once

#include <stdbool.h>
#include <stddef.h>

typedef enum llama_rs_status {
    LLAMA_RS_STATUS_OK = 0,
    LLAMA_RS_STATUS_INVALID_ARGUMENT = -1,
    LLAMA_RS_STATUS_ALLOCATION_FAILED = -2,
    LLAMA_RS_STATUS_EXCEPTION = -3
} llama_rs_status;

#ifdef __cplusplus

#include <cstdlib>
#include <cstring>
#include <string>

static inline char * llama_rs_dup_string(const std::string & value) {
    char * buffer = static_cast<char *>(std::malloc(value.size() + 1));
    if (!buffer) {
        return nullptr;
    }
    std::memcpy(buffer, value.data(), value.size());
    buffer[value.size()] = '\0';
    return buffer;
}

#endif
