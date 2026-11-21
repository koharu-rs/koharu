// koharu-torch: a C function for each ATen operator, in the generated
// koharu_torch_aten.h, and here what else libtorch's C++ API offers that
// libtorch's own C functions lack.
//
// A tensor is libtorch's AtenTensorHandle and a generator its
// AtenGeneratorHandle. libtorch's C functions in
// torch/csrc/inductor/aoti_torch/c/shim.h and torch/csrc/stable/c/shim.h
// make, inspect and delete them, and torch_exception_get_what gives the
// message of the last error a function here returned. A string or a byte
// string comes back as libtorch's StringHandle: read it with
// torch_string_c_str and torch_string_length, and free it with
// torch_delete_string. An array comes back from malloc: free it with
// aten_free, after taking or deleting the handles in it.

#pragma once

#include <torch/csrc/stable/c/shim.h>

#if defined(_WIN32)
#if defined(ATEN_BUILD)
#define ATEN_API __declspec(dllexport)
#else
#define ATEN_API __declspec(dllimport)
#endif
#else
#define ATEN_API __attribute__((visibility("default")))
#endif

#ifdef __cplusplus
extern "C" {
#endif

struct DLManagedTensor;
struct DLManagedTensorVersioned;

ATEN_API void aten_free(void* array);

// --- Bools and libtorch's enums ---------------------------------------------
// The generated functions pass a bool as int32_t, as libtorch's own generated
// C shim does, and an enum as its c10 value, the code aoti_torch_dtype_*()
// and kin return. These names say which a value is.

typedef int32_t AtenBool;          // 0 or 1
typedef int32_t AtenDtype;         // c10::ScalarType
typedef int32_t AtenLayout;        // c10::Layout
typedef int32_t AtenMemoryFormat;  // c10::MemoryFormat
typedef int32_t AtenQScheme;       // c10::QScheme
typedef int32_t AtenDeviceType;    // c10::DeviceType

// --- Scalars ----------------------------------------------------------------

typedef enum AtenScalarType {
  ATEN_SCALAR_BOOL,
  ATEN_SCALAR_INT,
  ATEN_SCALAR_FLOAT,
  ATEN_SCALAR_COMPLEX,
} AtenScalarType;

// An ATen Scalar with its type, which type promotion reads: an integer
// added to an integer tensor keeps it an integer tensor.
typedef struct AtenScalar {
  AtenScalarType type;
  union {
    bool b;
    int64_t i;
    double f;
    struct {
      double real;
      double imag;
    } c;
  } value;
} AtenScalar;

// --- Autograd ---------------------------------------------------------------
// Backward is the operator aten::_backward; requires_grad_, detach,
// retain_grad and is_leaf are operators too.

struct AtenInferenceModeOpaque;
typedef struct AtenInferenceModeOpaque* AtenInferenceModeHandle;

// Enters c10::InferenceMode on the calling thread until the guard is
// deleted there: no gradients, and none of autograd's per-operator
// bookkeeping. Guards on one thread are deleted in the reverse order.
ATEN_API AOTITorchError
aten_create_inference_mode(AtenInferenceModeHandle* ret);
ATEN_API AOTITorchError
aten_delete_inference_mode(AtenInferenceModeHandle guard);

ATEN_API AOTITorchError
aten_requires_grad(AtenTensorHandle tensor, bool* ret);

// The tensor's .grad, an undefined tensor when it has none.
ATEN_API AOTITorchError
aten_grad(AtenTensorHandle tensor, AtenTensorHandle* ret);

// Sets the tensor's .grad; NULL clears it.
ATEN_API AOTITorchError
aten_set_grad(AtenTensorHandle tensor, AtenTensorHandle grad);

// torch::autograd::grad: the gradients of outputs with respect to inputs,
// one per input, an undefined tensor for an unused one. grad_outputs may be
// NULL, and retain_graph NULL for its default.
ATEN_API AOTITorchError aten_autograd_grad(
    const AtenTensorHandle* outputs,
    int64_t outputs_len,
    const AtenTensorHandle* inputs,
    int64_t inputs_len,
    const AtenTensorHandle* grad_outputs,
    int64_t grad_outputs_len,
    const bool* retain_graph,
    bool create_graph,
    bool allow_unused,
    AtenTensorHandle** ret,
    int64_t* ret_len);

// --- Random numbers ---------------------------------------------------------
// Delete a generator with libtorch's torch_delete_generator.

// Seeds every device's default generator, as torch.manual_seed does.
ATEN_API AOTITorchError aten_manual_seed(uint64_t seed);

// A new generator for the device, as torch.Generator(device) makes.
ATEN_API AOTITorchError aten_create_generator(
    AtenDeviceType device_type,
    int32_t device_index,
    AtenGeneratorHandle* ret);

// The device's default generator, the one operators use without one.
ATEN_API AOTITorchError aten_default_generator(
    AtenDeviceType device_type,
    int32_t device_index,
    AtenGeneratorHandle* ret);

ATEN_API AOTITorchError
aten_generator_set_seed(AtenGeneratorHandle generator, uint64_t seed);
ATEN_API AOTITorchError
aten_generator_seed(AtenGeneratorHandle generator, uint64_t* ret);
ATEN_API AOTITorchError
aten_generator_set_offset(AtenGeneratorHandle generator, uint64_t offset);
ATEN_API AOTITorchError
aten_generator_offset(AtenGeneratorHandle generator, uint64_t* ret);
ATEN_API AOTITorchError aten_generator_state(
    AtenGeneratorHandle generator,
    AtenTensorHandle* ret);
ATEN_API AOTITorchError aten_generator_set_state(
    AtenGeneratorHandle generator,
    AtenTensorHandle state);

// --- Accelerator ------------------------------------------------------------
// The device libtorch was built for: CUDA (which a HIP build also reports),
// MPS or XPU, as torch.accelerator sees it.

// device_type is -1 when there's no accelerator, and then device_count is 0.
ATEN_API AOTITorchError aten_accelerator(
    AtenDeviceType* device_type,
    int32_t* device_count);
ATEN_API AOTITorchError aten_set_device(int32_t device_index);
ATEN_API AOTITorchError aten_synchronize(int32_t device_index);

// One of c10::CachingAllocator's statistics, for all its pools.
typedef struct AtenMemoryStat {
  int64_t current;
  int64_t peak;
  int64_t allocated;
  int64_t freed;
} AtenMemoryStat;

// The caching allocator's c10::CachingDeviceAllocator::DeviceStats.
typedef struct AtenMemoryStats {
  AtenMemoryStat allocation;
  AtenMemoryStat segment;
  AtenMemoryStat active;
  AtenMemoryStat inactive_split;
  AtenMemoryStat allocated_bytes;
  AtenMemoryStat reserved_bytes;
  AtenMemoryStat active_bytes;
  AtenMemoryStat inactive_split_bytes;
  AtenMemoryStat requested_bytes;
  int64_t num_alloc_retries;
  int64_t num_ooms;
  int64_t num_sync_all_streams;
  int64_t num_device_alloc;
  int64_t num_device_free;
} AtenMemoryStats;

ATEN_API AOTITorchError aten_memory_stats(
    int32_t device_index,
    AtenMemoryStats* ret);
ATEN_API AOTITorchError
aten_reset_peak_memory_stats(int32_t device_index);
ATEN_API AOTITorchError
aten_reset_accumulated_memory_stats(int32_t device_index);

// The device's free and total memory, in bytes.
ATEN_API AOTITorchError aten_memory_info(
    int32_t device_index,
    uint64_t* free,
    uint64_t* total);

// Returns the caching allocator's unused device memory to the device.
ATEN_API AOTITorchError aten_empty_cache(void);

// Returns the caching allocator's unused pinned host memory.
ATEN_API AOTITorchError aten_empty_host_cache(void);

// --- Streams and events -----------------------------------------------------

// A c10::Stream, by value.
typedef struct AtenStream {
  int64_t id;
  int32_t device_index;
  AtenDeviceType device_type;
} AtenStream;

ATEN_API AOTITorchError
aten_current_stream(int32_t device_index, AtenStream* ret);
ATEN_API AOTITorchError
aten_set_current_stream(AtenStream stream);

// A stream from the device's pool, as torch.Stream(device) takes.
ATEN_API AOTITorchError aten_stream_from_pool(
    AtenDeviceType device_type,
    int32_t device_index,
    bool high_priority,
    AtenStream* ret);
ATEN_API AOTITorchError
aten_stream_synchronize(AtenStream stream);

// Whether the stream has finished all its work.
ATEN_API AOTITorchError
aten_stream_query(AtenStream stream, bool* ret);

struct AtenEventOpaque;
typedef struct AtenEventOpaque* AtenEventHandle;

// A c10::Event. Only one with timing measures elapsed time.
ATEN_API AOTITorchError aten_create_event(
    AtenDeviceType device_type,
    bool enable_timing,
    AtenEventHandle* ret);
ATEN_API AOTITorchError
aten_delete_event(AtenEventHandle event);
ATEN_API AOTITorchError
aten_event_record(AtenEventHandle event, AtenStream stream);

// Makes the stream wait for the event.
ATEN_API AOTITorchError
aten_event_block(AtenEventHandle event, AtenStream stream);
ATEN_API AOTITorchError
aten_event_query(AtenEventHandle event, bool* ret);
ATEN_API AOTITorchError
aten_event_synchronize(AtenEventHandle event);

// Milliseconds from start to end.
ATEN_API AOTITorchError aten_event_elapsed_time(
    AtenEventHandle start,
    AtenEventHandle end,
    double* ret);

// --- Graphs -----------------------------------------------------------------
// at::accelerator::Graph: CUDA graphs on CUDA and HIP. Capture on a stream
// other than the device's default one.

struct AtenGraphOpaque;
typedef struct AtenGraphOpaque* AtenGraphHandle;

// A c10::MempoolId_t; {0, 0} lets a capture take a private pool.
typedef struct AtenMempoolId {
  uint64_t first;
  uint64_t second;
} AtenMempoolId;

// at::GraphCaptureMode's values.
typedef enum AtenGraphCaptureMode {
  ATEN_GRAPH_CAPTURE_DEFAULT,
  ATEN_GRAPH_CAPTURE_GLOBAL,
  ATEN_GRAPH_CAPTURE_THREAD_LOCAL,
  ATEN_GRAPH_CAPTURE_RELAXED,
} AtenGraphCaptureMode;

ATEN_API AOTITorchError
aten_create_graph(bool keep_graph, AtenGraphHandle* ret);
ATEN_API AOTITorchError
aten_delete_graph(AtenGraphHandle graph);
ATEN_API AOTITorchError aten_graph_capture_begin(
    AtenGraphHandle graph,
    AtenMempoolId pool,
    AtenGraphCaptureMode mode);
ATEN_API AOTITorchError
aten_graph_capture_end(AtenGraphHandle graph);
ATEN_API AOTITorchError
aten_graph_instantiate(AtenGraphHandle graph);
ATEN_API AOTITorchError
aten_graph_replay(AtenGraphHandle graph);
ATEN_API AOTITorchError
aten_graph_reset(AtenGraphHandle graph);

// The memory pool the graph captured into, to share with another capture.
ATEN_API AOTITorchError
aten_graph_pool(AtenGraphHandle graph, AtenMempoolId* ret);

// --- Autocast ---------------------------------------------------------------
// at::autocast, per device type, on the calling thread.

ATEN_API AOTITorchError aten_set_autocast_enabled(
    AtenDeviceType device_type,
    bool enabled);
ATEN_API AOTITorchError
aten_autocast_enabled(AtenDeviceType device_type, bool* ret);
ATEN_API AOTITorchError aten_set_autocast_dtype(
    AtenDeviceType device_type,
    AtenDtype dtype);
ATEN_API AOTITorchError aten_autocast_dtype(
    AtenDeviceType device_type,
    AtenDtype* ret);
ATEN_API AOTITorchError
aten_set_autocast_cache_enabled(bool enabled);
ATEN_API AOTITorchError aten_autocast_cache_enabled(bool* ret);

// The nesting depth after entering or leaving an autocast region; leaving
// the outermost one is when to clear the cache.
ATEN_API AOTITorchError aten_autocast_increment_nesting(int32_t* ret);
ATEN_API AOTITorchError aten_autocast_decrement_nesting(int32_t* ret);
ATEN_API AOTITorchError aten_autocast_clear_cache(void);

// --- Settings ---------------------------------------------------------------
// libtorch's global switches: torch.set_num_threads, torch.backends and
// torch.use_deterministic_algorithms. A switch is 0 or 1; an enum setting
// takes the libtorch enum's value.

typedef enum AtenSetting {
  ATEN_NUM_THREADS,
  ATEN_NUM_INTEROP_THREADS,
  // cuDNN's switches also turn MIOpen on and off on ROCm.
  ATEN_CUDNN_ENABLED,
  ATEN_CUDNN_BENCHMARK,
  ATEN_CUDNN_BENCHMARK_LIMIT,
  ATEN_CUDNN_DETERMINISTIC,
  ATEN_CUDNN_ALLOW_TF32,
  ATEN_CUBLAS_ALLOW_TF32,
  ATEN_CUBLAS_ALLOW_FP16_ACCUMULATION,
  // at::Float32MatmulPrecision: highest, high, medium.
  ATEN_FLOAT32_MATMUL_PRECISION,
  // at::BlasBackend, at::LinalgBackend and at::ROCmFABackend.
  ATEN_BLAS_BACKEND,
  ATEN_LINALG_BACKEND,
  ATEN_ROCM_FA_BACKEND,
  // The backends scaled_dot_product_attention may choose.
  ATEN_SDP_FLASH,
  ATEN_SDP_FA3,
  ATEN_SDP_FA4,
  ATEN_SDP_MEM_EFFICIENT,
  ATEN_SDP_MATH,
  ATEN_SDP_CUDNN,
  ATEN_SDP_OVERRIDEABLE,
  ATEN_SDP_MATH_FP16_BF16_REDUCTION,
  ATEN_MKLDNN_ENABLED,
  ATEN_MKLDNN_DETERMINISTIC,
  ATEN_NNPACK_ENABLED,
  ATEN_CPU_ALLOW_FP16_REDUCTION,
  ATEN_DETERMINISTIC_ALGORITHMS,
  ATEN_DETERMINISTIC_ALGORITHMS_WARN_ONLY,
  ATEN_DETERMINISTIC_FILL_UNINITIALIZED_MEMORY,
  // c10::QEngine.
  ATEN_QUANTIZED_ENGINE,
} AtenSetting;

ATEN_API AOTITorchError
aten_set_setting(AtenSetting setting, int64_t value);
ATEN_API AOTITorchError
aten_setting(AtenSetting setting, int64_t* ret);

// --- Build ------------------------------------------------------------------
// The libtorch version is TORCH_VERSION_* in torch/headeronly/version.h.

typedef enum AtenFeature {
  ATEN_HAS_CUDA,
  ATEN_HAS_HIP,
  ATEN_HAS_MPS,
  ATEN_HAS_XPU,
  ATEN_HAS_CUDNN,
  ATEN_HAS_MKL,
  ATEN_HAS_MKLDNN,
  ATEN_HAS_OPENMP,
  ATEN_HAS_LAPACK,
  ATEN_HAS_MAGMA,
  ATEN_HAS_KLEIDIAI,
} AtenFeature;

ATEN_API AOTITorchError
aten_has(AtenFeature feature, bool* ret);

typedef enum AtenLibrary {
  ATEN_CUDNN_VERSION,
  ATEN_CUDA_RUNTIME_VERSION,
  ATEN_HIPBLASLT_VERSION,
} AtenLibrary;

// A library's version as the CUDA hooks report it; an error without one.
ATEN_API AOTITorchError
aten_library_version(AtenLibrary library, int64_t* ret);

typedef enum AtenInfo {
  // torch.__config__.show().
  ATEN_CONFIG,
  // torch.__config__.parallel_info().
  ATEN_PARALLEL_INFO,
  ATEN_CPU_CAPABILITY,
  ATEN_MKL_VERSION,
  ATEN_MKLDNN_VERSION,
  ATEN_OPENMP_VERSION,
} AtenInfo;

ATEN_API AOTITorchError
aten_info(AtenInfo info, StringHandle* ret);

// --- Interchange ------------------------------------------------------------

// The tensor as libtorch's C++ prints it, std::cout << tensor.
ATEN_API AOTITorchError
aten_tensor_to_string(AtenTensorHandle tensor, StringHandle* ret);

// DLPack, sharing the tensor's memory. The capsule's deleter frees it.
ATEN_API AOTITorchError aten_to_dlpack(
    AtenTensorHandle tensor,
    struct DLManagedTensorVersioned** ret);
ATEN_API AOTITorchError aten_from_dlpack(
    struct DLManagedTensorVersioned* capsule,
    AtenTensorHandle* ret);
ATEN_API AOTITorchError aten_to_dlpack_unversioned(
    AtenTensorHandle tensor,
    struct DLManagedTensor** ret);
ATEN_API AOTITorchError aten_from_dlpack_unversioned(
    struct DLManagedTensor* capsule,
    AtenTensorHandle* ret);

// Named tensors as torch.save writes a dict of them; torch.load reads it.
ATEN_API AOTITorchError aten_save(
    const char* const* names,
    const AtenTensorHandle* tensors,
    int64_t len,
    StringHandle* ret);

// The tensors in bytes torch.save wrote: a dict of tensors by name, a list
// or tuple of them named by position, or one tensor named "".
ATEN_API AOTITorchError aten_load(
    const void* data,
    size_t size,
    StringHandle** names,
    AtenTensorHandle** tensors,
    int64_t* len);

// --- AOTInductor packages ---------------------------------------------------
// A model torch._inductor.aoti_compile_and_package compiled, as
// torch._inductor.aoti_load_package loads it.

struct AtenAotiPackageOpaque;
typedef struct AtenAotiPackageOpaque* AtenAotiPackageHandle;

ATEN_API AOTITorchError aten_load_aoti_package(
    const char* path,
    const char* model_name,
    bool run_single_threaded,
    int64_t num_runners,
    int32_t device_index,
    AtenAotiPackageHandle* ret);
ATEN_API AOTITorchError
aten_delete_aoti_package(AtenAotiPackageHandle package);

// Runs the model on stream, a backend stream such as cudaStream_t, or NULL
// for the current one.
ATEN_API AOTITorchError aten_aoti_package_run(
    AtenAotiPackageHandle package,
    const AtenTensorHandle* inputs,
    int64_t inputs_len,
    void* stream,
    AtenTensorHandle** ret,
    int64_t* ret_len);
ATEN_API AOTITorchError aten_aoti_package_metadata(
    AtenAotiPackageHandle package,
    StringHandle** keys,
    StringHandle** values,
    int64_t* len);

// The input and output pytree specs.
ATEN_API AOTITorchError aten_aoti_package_call_spec(
    AtenAotiPackageHandle package,
    StringHandle** ret,
    int64_t* ret_len);
ATEN_API AOTITorchError aten_aoti_package_constant_names(
    AtenAotiPackageHandle package,
    StringHandle** ret,
    int64_t* ret_len);
ATEN_API AOTITorchError aten_aoti_package_load_constants(
    AtenAotiPackageHandle package,
    const char* const* names,
    const AtenTensorHandle* tensors,
    int64_t len,
    bool use_inactive,
    bool check_full_update,
    bool user_managed);

// --- Warnings ---------------------------------------------------------------

typedef void (*AtenWarningHandler)(const char* message, void* context);

// Sends the calling thread's warnings to handler rather than stderr; NULL
// restores stderr. c10 keeps a warning handler per thread.
ATEN_API void aten_set_warning_handler(
    AtenWarningHandler handler,
    void* context);

#ifdef __cplusplus
} // extern "C"
#endif
