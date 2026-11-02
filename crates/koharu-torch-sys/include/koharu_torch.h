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
// koharu_torch_free, after taking or deleting the handles in it.

#pragma once

#include <torch/csrc/stable/c/shim.h>

#if defined(_WIN32)
#if defined(KOHARU_TORCH_BUILD)
#define KOHARU_TORCH_API __declspec(dllexport)
#else
#define KOHARU_TORCH_API __declspec(dllimport)
#endif
#else
#define KOHARU_TORCH_API __attribute__((visibility("default")))
#endif

#ifdef __cplusplus
extern "C" {
#endif

struct DLManagedTensor;
struct DLManagedTensorVersioned;

KOHARU_TORCH_API void koharu_torch_free(void* array);

// --- Bools and libtorch's enums ---------------------------------------------
// The generated functions pass a bool as int32_t, as libtorch's own generated
// C shim does, and an enum as its c10 value, the code aoti_torch_dtype_*()
// and kin return. These names say which a value is.

typedef int32_t KoharuTorchBool;          // 0 or 1
typedef int32_t KoharuTorchDtype;         // c10::ScalarType
typedef int32_t KoharuTorchLayout;        // c10::Layout
typedef int32_t KoharuTorchMemoryFormat;  // c10::MemoryFormat
typedef int32_t KoharuTorchQScheme;       // c10::QScheme
typedef int32_t KoharuTorchDeviceType;    // c10::DeviceType

// --- Scalars ----------------------------------------------------------------

typedef enum KoharuTorchScalarType {
  KOHARU_TORCH_SCALAR_BOOL,
  KOHARU_TORCH_SCALAR_INT,
  KOHARU_TORCH_SCALAR_FLOAT,
  KOHARU_TORCH_SCALAR_COMPLEX,
} KoharuTorchScalarType;

// An ATen Scalar with its type, which type promotion reads: an integer
// added to an integer tensor keeps it an integer tensor.
typedef struct KoharuTorchScalar {
  KoharuTorchScalarType type;
  union {
    bool b;
    int64_t i;
    double f;
    struct {
      double real;
      double imag;
    } c;
  } value;
} KoharuTorchScalar;

// --- Autograd ---------------------------------------------------------------
// Backward is the operator aten::_backward; requires_grad_, detach,
// retain_grad and is_leaf are operators too.

struct KoharuTorchInferenceModeOpaque;
typedef struct KoharuTorchInferenceModeOpaque* KoharuTorchInferenceModeHandle;

// Enters c10::InferenceMode on the calling thread until the guard is
// deleted there: no gradients, and none of autograd's per-operator
// bookkeeping. Guards on one thread are deleted in the reverse order.
KOHARU_TORCH_API AOTITorchError
koharu_torch_create_inference_mode(KoharuTorchInferenceModeHandle* ret);
KOHARU_TORCH_API AOTITorchError
koharu_torch_delete_inference_mode(KoharuTorchInferenceModeHandle guard);

KOHARU_TORCH_API AOTITorchError
koharu_torch_requires_grad(AtenTensorHandle tensor, bool* ret);

// The tensor's .grad, an undefined tensor when it has none.
KOHARU_TORCH_API AOTITorchError
koharu_torch_grad(AtenTensorHandle tensor, AtenTensorHandle* ret);

// Sets the tensor's .grad; NULL clears it.
KOHARU_TORCH_API AOTITorchError
koharu_torch_set_grad(AtenTensorHandle tensor, AtenTensorHandle grad);

// torch::autograd::grad: the gradients of outputs with respect to inputs,
// one per input, an undefined tensor for an unused one. grad_outputs may be
// NULL, and retain_graph NULL for its default.
KOHARU_TORCH_API AOTITorchError koharu_torch_autograd_grad(
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
KOHARU_TORCH_API AOTITorchError koharu_torch_manual_seed(uint64_t seed);

// A new generator for the device, as torch.Generator(device) makes.
KOHARU_TORCH_API AOTITorchError koharu_torch_create_generator(
    KoharuTorchDeviceType device_type,
    int32_t device_index,
    AtenGeneratorHandle* ret);

// The device's default generator, the one operators use without one.
KOHARU_TORCH_API AOTITorchError koharu_torch_default_generator(
    KoharuTorchDeviceType device_type,
    int32_t device_index,
    AtenGeneratorHandle* ret);

KOHARU_TORCH_API AOTITorchError
koharu_torch_generator_set_seed(AtenGeneratorHandle generator, uint64_t seed);
KOHARU_TORCH_API AOTITorchError
koharu_torch_generator_seed(AtenGeneratorHandle generator, uint64_t* ret);
KOHARU_TORCH_API AOTITorchError
koharu_torch_generator_set_offset(AtenGeneratorHandle generator, uint64_t offset);
KOHARU_TORCH_API AOTITorchError
koharu_torch_generator_offset(AtenGeneratorHandle generator, uint64_t* ret);
KOHARU_TORCH_API AOTITorchError koharu_torch_generator_state(
    AtenGeneratorHandle generator,
    AtenTensorHandle* ret);
KOHARU_TORCH_API AOTITorchError koharu_torch_generator_set_state(
    AtenGeneratorHandle generator,
    AtenTensorHandle state);

// --- Accelerator ------------------------------------------------------------
// The device libtorch was built for: CUDA (which a HIP build also reports),
// MPS or XPU, as torch.accelerator sees it.

// device_type is -1 when there's no accelerator, and then device_count is 0.
KOHARU_TORCH_API AOTITorchError koharu_torch_accelerator(
    KoharuTorchDeviceType* device_type,
    int32_t* device_count);
KOHARU_TORCH_API AOTITorchError koharu_torch_set_device(int32_t device_index);
KOHARU_TORCH_API AOTITorchError koharu_torch_synchronize(int32_t device_index);

// One of c10::CachingAllocator's statistics, for all its pools.
typedef struct KoharuTorchMemoryStat {
  int64_t current;
  int64_t peak;
  int64_t allocated;
  int64_t freed;
} KoharuTorchMemoryStat;

// The caching allocator's c10::CachingDeviceAllocator::DeviceStats.
typedef struct KoharuTorchMemoryStats {
  KoharuTorchMemoryStat allocation;
  KoharuTorchMemoryStat segment;
  KoharuTorchMemoryStat active;
  KoharuTorchMemoryStat inactive_split;
  KoharuTorchMemoryStat allocated_bytes;
  KoharuTorchMemoryStat reserved_bytes;
  KoharuTorchMemoryStat active_bytes;
  KoharuTorchMemoryStat inactive_split_bytes;
  KoharuTorchMemoryStat requested_bytes;
  int64_t num_alloc_retries;
  int64_t num_ooms;
  int64_t num_sync_all_streams;
  int64_t num_device_alloc;
  int64_t num_device_free;
} KoharuTorchMemoryStats;

KOHARU_TORCH_API AOTITorchError koharu_torch_memory_stats(
    int32_t device_index,
    KoharuTorchMemoryStats* ret);
KOHARU_TORCH_API AOTITorchError
koharu_torch_reset_peak_memory_stats(int32_t device_index);
KOHARU_TORCH_API AOTITorchError
koharu_torch_reset_accumulated_memory_stats(int32_t device_index);

// The device's free and total memory, in bytes.
KOHARU_TORCH_API AOTITorchError koharu_torch_memory_info(
    int32_t device_index,
    uint64_t* free,
    uint64_t* total);

// Returns the caching allocator's unused device memory to the device.
KOHARU_TORCH_API AOTITorchError koharu_torch_empty_cache(void);

// Returns the caching allocator's unused pinned host memory.
KOHARU_TORCH_API AOTITorchError koharu_torch_empty_host_cache(void);

// --- Streams and events -----------------------------------------------------

// A c10::Stream, by value.
typedef struct KoharuTorchStream {
  int64_t id;
  int32_t device_index;
  KoharuTorchDeviceType device_type;
} KoharuTorchStream;

KOHARU_TORCH_API AOTITorchError
koharu_torch_current_stream(int32_t device_index, KoharuTorchStream* ret);
KOHARU_TORCH_API AOTITorchError
koharu_torch_set_current_stream(KoharuTorchStream stream);

// A stream from the device's pool, as torch.Stream(device) takes.
KOHARU_TORCH_API AOTITorchError koharu_torch_stream_from_pool(
    KoharuTorchDeviceType device_type,
    int32_t device_index,
    bool high_priority,
    KoharuTorchStream* ret);
KOHARU_TORCH_API AOTITorchError
koharu_torch_stream_synchronize(KoharuTorchStream stream);

// Whether the stream has finished all its work.
KOHARU_TORCH_API AOTITorchError
koharu_torch_stream_query(KoharuTorchStream stream, bool* ret);

struct KoharuTorchEventOpaque;
typedef struct KoharuTorchEventOpaque* KoharuTorchEventHandle;

// A c10::Event. Only one with timing measures elapsed time.
KOHARU_TORCH_API AOTITorchError koharu_torch_create_event(
    KoharuTorchDeviceType device_type,
    bool enable_timing,
    KoharuTorchEventHandle* ret);
KOHARU_TORCH_API AOTITorchError
koharu_torch_delete_event(KoharuTorchEventHandle event);
KOHARU_TORCH_API AOTITorchError
koharu_torch_event_record(KoharuTorchEventHandle event, KoharuTorchStream stream);

// Makes the stream wait for the event.
KOHARU_TORCH_API AOTITorchError
koharu_torch_event_block(KoharuTorchEventHandle event, KoharuTorchStream stream);
KOHARU_TORCH_API AOTITorchError
koharu_torch_event_query(KoharuTorchEventHandle event, bool* ret);
KOHARU_TORCH_API AOTITorchError
koharu_torch_event_synchronize(KoharuTorchEventHandle event);

// Milliseconds from start to end.
KOHARU_TORCH_API AOTITorchError koharu_torch_event_elapsed_time(
    KoharuTorchEventHandle start,
    KoharuTorchEventHandle end,
    double* ret);

// --- Graphs -----------------------------------------------------------------
// at::accelerator::Graph: CUDA graphs on CUDA and HIP. Capture on a stream
// other than the device's default one.

struct KoharuTorchGraphOpaque;
typedef struct KoharuTorchGraphOpaque* KoharuTorchGraphHandle;

// A c10::MempoolId_t; {0, 0} lets a capture take a private pool.
typedef struct KoharuTorchMempoolId {
  uint64_t first;
  uint64_t second;
} KoharuTorchMempoolId;

// at::GraphCaptureMode's values.
typedef enum KoharuTorchGraphCaptureMode {
  KOHARU_TORCH_GRAPH_CAPTURE_DEFAULT,
  KOHARU_TORCH_GRAPH_CAPTURE_GLOBAL,
  KOHARU_TORCH_GRAPH_CAPTURE_THREAD_LOCAL,
  KOHARU_TORCH_GRAPH_CAPTURE_RELAXED,
} KoharuTorchGraphCaptureMode;

KOHARU_TORCH_API AOTITorchError
koharu_torch_create_graph(bool keep_graph, KoharuTorchGraphHandle* ret);
KOHARU_TORCH_API AOTITorchError
koharu_torch_delete_graph(KoharuTorchGraphHandle graph);
KOHARU_TORCH_API AOTITorchError koharu_torch_graph_capture_begin(
    KoharuTorchGraphHandle graph,
    KoharuTorchMempoolId pool,
    KoharuTorchGraphCaptureMode mode);
KOHARU_TORCH_API AOTITorchError
koharu_torch_graph_capture_end(KoharuTorchGraphHandle graph);
KOHARU_TORCH_API AOTITorchError
koharu_torch_graph_instantiate(KoharuTorchGraphHandle graph);
KOHARU_TORCH_API AOTITorchError
koharu_torch_graph_replay(KoharuTorchGraphHandle graph);
KOHARU_TORCH_API AOTITorchError
koharu_torch_graph_reset(KoharuTorchGraphHandle graph);

// The memory pool the graph captured into, to share with another capture.
KOHARU_TORCH_API AOTITorchError
koharu_torch_graph_pool(KoharuTorchGraphHandle graph, KoharuTorchMempoolId* ret);

// --- Autocast ---------------------------------------------------------------
// at::autocast, per device type, on the calling thread.

KOHARU_TORCH_API AOTITorchError koharu_torch_set_autocast_enabled(
    KoharuTorchDeviceType device_type,
    bool enabled);
KOHARU_TORCH_API AOTITorchError
koharu_torch_autocast_enabled(KoharuTorchDeviceType device_type, bool* ret);
KOHARU_TORCH_API AOTITorchError koharu_torch_set_autocast_dtype(
    KoharuTorchDeviceType device_type,
    KoharuTorchDtype dtype);
KOHARU_TORCH_API AOTITorchError koharu_torch_autocast_dtype(
    KoharuTorchDeviceType device_type,
    KoharuTorchDtype* ret);
KOHARU_TORCH_API AOTITorchError
koharu_torch_set_autocast_cache_enabled(bool enabled);
KOHARU_TORCH_API AOTITorchError koharu_torch_autocast_cache_enabled(bool* ret);

// The nesting depth after entering or leaving an autocast region; leaving
// the outermost one is when to clear the cache.
KOHARU_TORCH_API AOTITorchError koharu_torch_autocast_increment_nesting(int32_t* ret);
KOHARU_TORCH_API AOTITorchError koharu_torch_autocast_decrement_nesting(int32_t* ret);
KOHARU_TORCH_API AOTITorchError koharu_torch_autocast_clear_cache(void);

// --- Settings ---------------------------------------------------------------
// libtorch's global switches: torch.set_num_threads, torch.backends and
// torch.use_deterministic_algorithms. A switch is 0 or 1; an enum setting
// takes the libtorch enum's value.

typedef enum KoharuTorchSetting {
  KOHARU_TORCH_NUM_THREADS,
  KOHARU_TORCH_NUM_INTEROP_THREADS,
  // cuDNN's switches also turn MIOpen on and off on ROCm.
  KOHARU_TORCH_CUDNN_ENABLED,
  KOHARU_TORCH_CUDNN_BENCHMARK,
  KOHARU_TORCH_CUDNN_BENCHMARK_LIMIT,
  KOHARU_TORCH_CUDNN_DETERMINISTIC,
  KOHARU_TORCH_CUDNN_ALLOW_TF32,
  KOHARU_TORCH_CUBLAS_ALLOW_TF32,
  KOHARU_TORCH_CUBLAS_ALLOW_FP16_ACCUMULATION,
  // at::Float32MatmulPrecision: highest, high, medium.
  KOHARU_TORCH_FLOAT32_MATMUL_PRECISION,
  // at::BlasBackend, at::LinalgBackend and at::ROCmFABackend.
  KOHARU_TORCH_BLAS_BACKEND,
  KOHARU_TORCH_LINALG_BACKEND,
  KOHARU_TORCH_ROCM_FA_BACKEND,
  // The backends scaled_dot_product_attention may choose.
  KOHARU_TORCH_SDP_FLASH,
  KOHARU_TORCH_SDP_FA3,
  KOHARU_TORCH_SDP_FA4,
  KOHARU_TORCH_SDP_MEM_EFFICIENT,
  KOHARU_TORCH_SDP_MATH,
  KOHARU_TORCH_SDP_CUDNN,
  KOHARU_TORCH_SDP_OVERRIDEABLE,
  KOHARU_TORCH_SDP_MATH_FP16_BF16_REDUCTION,
  KOHARU_TORCH_MKLDNN_ENABLED,
  KOHARU_TORCH_MKLDNN_DETERMINISTIC,
  KOHARU_TORCH_NNPACK_ENABLED,
  KOHARU_TORCH_CPU_ALLOW_FP16_REDUCTION,
  KOHARU_TORCH_DETERMINISTIC_ALGORITHMS,
  KOHARU_TORCH_DETERMINISTIC_ALGORITHMS_WARN_ONLY,
  KOHARU_TORCH_DETERMINISTIC_FILL_UNINITIALIZED_MEMORY,
  // c10::QEngine.
  KOHARU_TORCH_QUANTIZED_ENGINE,
} KoharuTorchSetting;

KOHARU_TORCH_API AOTITorchError
koharu_torch_set_setting(KoharuTorchSetting setting, int64_t value);
KOHARU_TORCH_API AOTITorchError
koharu_torch_setting(KoharuTorchSetting setting, int64_t* ret);

// --- Build ------------------------------------------------------------------
// The libtorch version is TORCH_VERSION_* in torch/headeronly/version.h.

typedef enum KoharuTorchFeature {
  KOHARU_TORCH_HAS_CUDA,
  KOHARU_TORCH_HAS_HIP,
  KOHARU_TORCH_HAS_MPS,
  KOHARU_TORCH_HAS_XPU,
  KOHARU_TORCH_HAS_CUDNN,
  KOHARU_TORCH_HAS_MKL,
  KOHARU_TORCH_HAS_MKLDNN,
  KOHARU_TORCH_HAS_OPENMP,
  KOHARU_TORCH_HAS_LAPACK,
  KOHARU_TORCH_HAS_MAGMA,
  KOHARU_TORCH_HAS_KLEIDIAI,
} KoharuTorchFeature;

KOHARU_TORCH_API AOTITorchError
koharu_torch_has(KoharuTorchFeature feature, bool* ret);

typedef enum KoharuTorchLibrary {
  KOHARU_TORCH_CUDNN_VERSION,
  KOHARU_TORCH_CUDA_RUNTIME_VERSION,
  KOHARU_TORCH_HIPBLASLT_VERSION,
} KoharuTorchLibrary;

// A library's version as the CUDA hooks report it; an error without one.
KOHARU_TORCH_API AOTITorchError
koharu_torch_library_version(KoharuTorchLibrary library, int64_t* ret);

typedef enum KoharuTorchInfo {
  // torch.__config__.show().
  KOHARU_TORCH_CONFIG,
  // torch.__config__.parallel_info().
  KOHARU_TORCH_PARALLEL_INFO,
  KOHARU_TORCH_CPU_CAPABILITY,
  KOHARU_TORCH_MKL_VERSION,
  KOHARU_TORCH_MKLDNN_VERSION,
  KOHARU_TORCH_OPENMP_VERSION,
} KoharuTorchInfo;

KOHARU_TORCH_API AOTITorchError
koharu_torch_info(KoharuTorchInfo info, StringHandle* ret);

// --- Interchange ------------------------------------------------------------

// The tensor as libtorch's C++ prints it, std::cout << tensor.
KOHARU_TORCH_API AOTITorchError
koharu_torch_tensor_to_string(AtenTensorHandle tensor, StringHandle* ret);

// DLPack, sharing the tensor's memory. The capsule's deleter frees it.
KOHARU_TORCH_API AOTITorchError koharu_torch_to_dlpack(
    AtenTensorHandle tensor,
    struct DLManagedTensorVersioned** ret);
KOHARU_TORCH_API AOTITorchError koharu_torch_from_dlpack(
    struct DLManagedTensorVersioned* capsule,
    AtenTensorHandle* ret);
KOHARU_TORCH_API AOTITorchError koharu_torch_to_dlpack_unversioned(
    AtenTensorHandle tensor,
    struct DLManagedTensor** ret);
KOHARU_TORCH_API AOTITorchError koharu_torch_from_dlpack_unversioned(
    struct DLManagedTensor* capsule,
    AtenTensorHandle* ret);

// Named tensors as torch.save writes a dict of them; torch.load reads it.
KOHARU_TORCH_API AOTITorchError koharu_torch_save(
    const char* const* names,
    const AtenTensorHandle* tensors,
    int64_t len,
    StringHandle* ret);

// The tensors in bytes torch.save wrote: a dict of tensors by name, a list
// or tuple of them named by position, or one tensor named "".
KOHARU_TORCH_API AOTITorchError koharu_torch_load(
    const void* data,
    size_t size,
    StringHandle** names,
    AtenTensorHandle** tensors,
    int64_t* len);

// --- AOTInductor packages ---------------------------------------------------
// A model torch._inductor.aoti_compile_and_package compiled, as
// torch._inductor.aoti_load_package loads it.

struct KoharuTorchAotiPackageOpaque;
typedef struct KoharuTorchAotiPackageOpaque* KoharuTorchAotiPackageHandle;

KOHARU_TORCH_API AOTITorchError koharu_torch_load_aoti_package(
    const char* path,
    const char* model_name,
    bool run_single_threaded,
    int64_t num_runners,
    int32_t device_index,
    KoharuTorchAotiPackageHandle* ret);
KOHARU_TORCH_API AOTITorchError
koharu_torch_delete_aoti_package(KoharuTorchAotiPackageHandle package);

// Runs the model on stream, a backend stream such as cudaStream_t, or NULL
// for the current one.
KOHARU_TORCH_API AOTITorchError koharu_torch_aoti_package_run(
    KoharuTorchAotiPackageHandle package,
    const AtenTensorHandle* inputs,
    int64_t inputs_len,
    void* stream,
    AtenTensorHandle** ret,
    int64_t* ret_len);
KOHARU_TORCH_API AOTITorchError koharu_torch_aoti_package_metadata(
    KoharuTorchAotiPackageHandle package,
    StringHandle** keys,
    StringHandle** values,
    int64_t* len);

// The input and output pytree specs.
KOHARU_TORCH_API AOTITorchError koharu_torch_aoti_package_call_spec(
    KoharuTorchAotiPackageHandle package,
    StringHandle** ret,
    int64_t* ret_len);
KOHARU_TORCH_API AOTITorchError koharu_torch_aoti_package_constant_names(
    KoharuTorchAotiPackageHandle package,
    StringHandle** ret,
    int64_t* ret_len);
KOHARU_TORCH_API AOTITorchError koharu_torch_aoti_package_load_constants(
    KoharuTorchAotiPackageHandle package,
    const char* const* names,
    const AtenTensorHandle* tensors,
    int64_t len,
    bool use_inactive,
    bool check_full_update,
    bool user_managed);

// --- Warnings ---------------------------------------------------------------

typedef void (*KoharuTorchWarningHandler)(const char* message, void* context);

// Sends the calling thread's warnings to handler rather than stderr; NULL
// restores stderr. c10 keeps a warning handler per thread.
KOHARU_TORCH_API void koharu_torch_set_warning_handler(
    KoharuTorchWarningHandler handler,
    void* context);

#ifdef __cplusplus
} // extern "C"
#endif
