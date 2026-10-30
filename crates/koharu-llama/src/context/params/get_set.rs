use std::num::NonZeroU32;

use super::{
    KvCacheType, LlamaAttentionType, LlamaContextParams, LlamaContextType, LlamaPoolingType,
    RopeScalingType,
};

impl LlamaContextParams {
    /// Set the size of the context
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use std::num::NonZeroU32;
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// let params = params.with_n_ctx(NonZeroU32::new(2048));
    /// assert_eq!(params.n_ctx(), NonZeroU32::new(2048));
    /// ```
    #[must_use]
    pub fn with_n_ctx(mut self, n_ctx: Option<NonZeroU32>) -> Self {
        self.context_params.n_ctx = n_ctx.map_or(0, std::num::NonZeroU32::get);
        self
    }

    /// Get the size of the context.
    ///
    /// [`None`] if the context size is specified by the model and not the context.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// assert_eq!(params.n_ctx(), std::num::NonZeroU32::new(512));
    /// ```
    #[must_use]
    pub fn n_ctx(&self) -> Option<NonZeroU32> {
        NonZeroU32::new(self.context_params.n_ctx)
    }

    /// Set the `n_batch`
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default()
    ///     .with_n_batch(2048);
    /// assert_eq!(params.n_batch(), 2048);
    /// ```
    #[must_use]
    pub fn with_n_batch(mut self, n_batch: u32) -> Self {
        self.context_params.n_batch = n_batch;
        self
    }

    /// Get the `n_batch`
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// assert_eq!(params.n_batch(), 2048);
    /// ```
    #[must_use]
    pub fn n_batch(&self) -> u32 {
        self.context_params.n_batch
    }

    /// Set the `n_ubatch`
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default()
    ///     .with_n_ubatch(512);
    /// assert_eq!(params.n_ubatch(), 512);
    /// ```
    #[must_use]
    pub fn with_n_ubatch(mut self, n_ubatch: u32) -> Self {
        self.context_params.n_ubatch = n_ubatch;
        self
    }

    /// Get the `n_ubatch`
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// assert_eq!(params.n_ubatch(), 512);
    /// ```
    #[must_use]
    pub fn n_ubatch(&self) -> u32 {
        self.context_params.n_ubatch
    }

    /// Set the max number of sequences (i.e. distinct states for recurrent models)
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default()
    ///     .with_n_seq_max(64);
    /// assert_eq!(params.n_seq_max(), 64);
    /// ```
    #[must_use]
    pub fn with_n_seq_max(mut self, n_seq_max: u32) -> Self {
        self.context_params.n_seq_max = n_seq_max;
        self
    }

    /// Get the max number of sequences (i.e. distinct states for recurrent models)
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// assert_eq!(params.n_seq_max(), 1);
    /// ```
    #[must_use]
    pub fn n_seq_max(&self) -> u32 {
        self.context_params.n_seq_max
    }

    /// Set the number of recurrent-state rollback snapshots per sequence.
    ///
    /// MTP speculative decoding uses this on the target context so llama.cpp can
    /// roll recurrent state back after partially accepted drafts.
    #[must_use]
    pub fn with_n_rs_seq(mut self, n_rs_seq: u32) -> Self {
        self.context_params.n_rs_seq = n_rs_seq;
        self
    }

    /// Get the number of recurrent-state rollback snapshots per sequence.
    #[must_use]
    pub fn n_rs_seq(&self) -> u32 {
        self.context_params.n_rs_seq
    }

    /// Set the llama.cpp context type.
    #[must_use]
    pub fn with_context_type(mut self, context_type: LlamaContextType) -> Self {
        self.context_params.ctx_type = context_type.into();
        self
    }

    /// Get the llama.cpp context type.
    #[must_use]
    pub fn context_type(&self) -> LlamaContextType {
        self.context_params.ctx_type.into()
    }

    /// Set the number of threads
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default()
    ///    .with_n_threads(8);
    /// assert_eq!(params.n_threads(), 8);
    /// ```
    #[must_use]
    pub fn with_n_threads(mut self, n_threads: i32) -> Self {
        self.context_params.n_threads = n_threads;
        self
    }

    /// Get the number of threads
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// assert_eq!(params.n_threads(), 4);
    /// ```
    #[must_use]
    pub fn n_threads(&self) -> i32 {
        self.context_params.n_threads
    }

    /// Set the number of threads allocated for batches
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default()
    ///    .with_n_threads_batch(8);
    /// assert_eq!(params.n_threads_batch(), 8);
    /// ```
    #[must_use]
    pub fn with_n_threads_batch(mut self, n_threads: i32) -> Self {
        self.context_params.n_threads_batch = n_threads;
        self
    }

    /// Get the number of threads allocated for batches
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// assert_eq!(params.n_threads_batch(), 4);
    /// ```
    #[must_use]
    pub fn n_threads_batch(&self) -> i32 {
        self.context_params.n_threads_batch
    }

    /// Set the type of rope scaling
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::{LlamaContextParams, RopeScalingType};
    /// let params = LlamaContextParams::default()
    ///     .with_rope_scaling_type(RopeScalingType::Linear);
    /// assert_eq!(params.rope_scaling_type(), RopeScalingType::Linear);
    /// ```
    #[must_use]
    pub fn with_rope_scaling_type(mut self, rope_scaling_type: RopeScalingType) -> Self {
        self.context_params.rope_scaling_type = i32::from(rope_scaling_type);
        self
    }

    /// Get the type of rope scaling
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::{LlamaContextParams, RopeScalingType};
    /// let params = LlamaContextParams::default();
    /// assert_eq!(params.rope_scaling_type(), RopeScalingType::Unspecified);
    /// ```
    #[must_use]
    pub fn rope_scaling_type(&self) -> RopeScalingType {
        RopeScalingType::from(self.context_params.rope_scaling_type)
    }

    /// Set the type of pooling
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::{LlamaContextParams, LlamaPoolingType};
    /// let params = LlamaContextParams::default()
    ///     .with_pooling_type(LlamaPoolingType::Last);
    /// assert_eq!(params.pooling_type(), LlamaPoolingType::Last);
    /// ```
    #[must_use]
    pub fn with_pooling_type(mut self, pooling_type: LlamaPoolingType) -> Self {
        self.context_params.pooling_type = i32::from(pooling_type);
        self
    }

    /// Get the type of pooling
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::{LlamaContextParams, LlamaPoolingType};
    /// let params = LlamaContextParams::default();
    /// assert_eq!(params.pooling_type(), LlamaPoolingType::Unspecified);
    /// ```
    #[must_use]
    pub fn pooling_type(&self) -> LlamaPoolingType {
        LlamaPoolingType::from(self.context_params.pooling_type)
    }

    /// Set the attention type for embeddings
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::{LlamaContextParams, LlamaAttentionType};
    /// let params = LlamaContextParams::default()
    ///     .with_attention_type(LlamaAttentionType::Causal);
    /// assert_eq!(params.attention_type(), LlamaAttentionType::Causal);
    /// ```
    #[must_use]
    pub fn with_attention_type(mut self, attention_type: LlamaAttentionType) -> Self {
        self.context_params.attention_type = i32::from(attention_type);
        self
    }

    /// Get the attention type for embeddings
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::{LlamaContextParams, LlamaAttentionType};
    /// let params = LlamaContextParams::default();
    /// assert_eq!(params.attention_type(), LlamaAttentionType::Unspecified);
    /// ```
    #[must_use]
    pub fn attention_type(&self) -> LlamaAttentionType {
        LlamaAttentionType::from(self.context_params.attention_type)
    }

    /// Set the flash attention policy using llama.cpp enum
    #[must_use]
    pub fn with_flash_attention_policy(
        mut self,
        policy: koharu_llama_sys::llama_flash_attn_type,
    ) -> Self {
        self.context_params.flash_attn_type = policy;
        self
    }

    /// Get the flash attention policy
    #[must_use]
    pub fn flash_attention_policy(&self) -> koharu_llama_sys::llama_flash_attn_type {
        self.context_params.flash_attn_type
    }

    /// Set the rope frequency base
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default()
    ///    .with_rope_freq_base(0.5);
    /// assert_eq!(params.rope_freq_base(), 0.5);
    /// ```
    #[must_use]
    pub fn with_rope_freq_base(mut self, rope_freq_base: f32) -> Self {
        self.context_params.rope_freq_base = rope_freq_base;
        self
    }

    /// Get the rope frequency base
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// assert_eq!(params.rope_freq_base(), 0.0);
    /// ```
    #[must_use]
    pub fn rope_freq_base(&self) -> f32 {
        self.context_params.rope_freq_base
    }

    /// Set the rope frequency scale
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default()
    ///   .with_rope_freq_scale(0.5);
    /// assert_eq!(params.rope_freq_scale(), 0.5);
    /// ```
    #[must_use]
    pub fn with_rope_freq_scale(mut self, rope_freq_scale: f32) -> Self {
        self.context_params.rope_freq_scale = rope_freq_scale;
        self
    }

    /// Get the rope frequency scale
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// assert_eq!(params.rope_freq_scale(), 0.0);
    /// ```
    #[must_use]
    pub fn rope_freq_scale(&self) -> f32 {
        self.context_params.rope_freq_scale
    }

    /// Set the YaRN extrapolation mix factor
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default().with_yarn_ext_factor(1.0);
    /// assert_eq!(params.yarn_ext_factor(), 1.0);
    /// ```
    #[must_use]
    pub fn with_yarn_ext_factor(mut self, yarn_ext_factor: f32) -> Self {
        self.context_params.yarn_ext_factor = yarn_ext_factor;
        self
    }

    /// Get the YaRN extrapolation mix factor
    #[must_use]
    pub fn yarn_ext_factor(&self) -> f32 {
        self.context_params.yarn_ext_factor
    }

    /// Set the YaRN magnitude scaling factor
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default().with_yarn_attn_factor(2.0);
    /// assert_eq!(params.yarn_attn_factor(), 2.0);
    /// ```
    #[must_use]
    pub fn with_yarn_attn_factor(mut self, yarn_attn_factor: f32) -> Self {
        self.context_params.yarn_attn_factor = yarn_attn_factor;
        self
    }

    /// Get the YaRN magnitude scaling factor
    #[must_use]
    pub fn yarn_attn_factor(&self) -> f32 {
        self.context_params.yarn_attn_factor
    }

    /// Set the YaRN low correction dim
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default().with_yarn_beta_fast(16.0);
    /// assert_eq!(params.yarn_beta_fast(), 16.0);
    /// ```
    #[must_use]
    pub fn with_yarn_beta_fast(mut self, yarn_beta_fast: f32) -> Self {
        self.context_params.yarn_beta_fast = yarn_beta_fast;
        self
    }

    /// Get the YaRN low correction dim
    #[must_use]
    pub fn yarn_beta_fast(&self) -> f32 {
        self.context_params.yarn_beta_fast
    }

    /// Set the YaRN high correction dim
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default().with_yarn_beta_slow(2.0);
    /// assert_eq!(params.yarn_beta_slow(), 2.0);
    /// ```
    #[must_use]
    pub fn with_yarn_beta_slow(mut self, yarn_beta_slow: f32) -> Self {
        self.context_params.yarn_beta_slow = yarn_beta_slow;
        self
    }

    /// Get the YaRN high correction dim
    #[must_use]
    pub fn yarn_beta_slow(&self) -> f32 {
        self.context_params.yarn_beta_slow
    }

    /// Set the YaRN original context size
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default().with_yarn_orig_ctx(4096);
    /// assert_eq!(params.yarn_orig_ctx(), 4096);
    /// ```
    #[must_use]
    pub fn with_yarn_orig_ctx(mut self, yarn_orig_ctx: u32) -> Self {
        self.context_params.yarn_orig_ctx = yarn_orig_ctx;
        self
    }

    /// Get the YaRN original context size
    #[must_use]
    pub fn yarn_orig_ctx(&self) -> u32 {
        self.context_params.yarn_orig_ctx
    }

    /// Set the KV cache defragmentation threshold
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default().with_defrag_thold(0.1);
    /// assert_eq!(params.defrag_thold(), 0.1);
    /// ```
    #[must_use]
    pub fn with_defrag_thold(mut self, defrag_thold: f32) -> Self {
        self.context_params.defrag_thold = defrag_thold;
        self
    }

    /// Get the KV cache defragmentation threshold
    #[must_use]
    pub fn defrag_thold(&self) -> f32 {
        self.context_params.defrag_thold
    }

    /// Set the KV cache data type for K
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::{LlamaContextParams, KvCacheType};
    /// let params = LlamaContextParams::default().with_type_k(KvCacheType::Q4_0);
    /// assert_eq!(params.type_k(), KvCacheType::Q4_0);
    /// ```
    #[must_use]
    pub fn with_type_k(mut self, type_k: KvCacheType) -> Self {
        self.context_params.type_k = type_k.into();
        self
    }

    /// Get the KV cache data type for K
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// let _ = params.type_k();
    /// ```
    #[must_use]
    pub fn type_k(&self) -> KvCacheType {
        KvCacheType::from(self.context_params.type_k)
    }

    /// Set the KV cache data type for V
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::{LlamaContextParams, KvCacheType};
    /// let params = LlamaContextParams::default().with_type_v(KvCacheType::Q4_1);
    /// assert_eq!(params.type_v(), KvCacheType::Q4_1);
    /// ```
    #[must_use]
    pub fn with_type_v(mut self, type_v: KvCacheType) -> Self {
        self.context_params.type_v = type_v.into();
        self
    }

    /// Get the KV cache data type for V
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// let _ = params.type_v();
    /// ```
    #[must_use]
    pub fn type_v(&self) -> KvCacheType {
        KvCacheType::from(self.context_params.type_v)
    }

    /// Set whether embeddings are enabled
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default()
    ///    .with_embeddings(true);
    /// assert!(params.embeddings());
    /// ```
    #[must_use]
    pub fn with_embeddings(mut self, embedding: bool) -> Self {
        self.context_params.embeddings = embedding;
        self
    }

    /// Get whether embeddings are enabled
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// assert!(!params.embeddings());
    /// ```
    #[must_use]
    pub fn embeddings(&self) -> bool {
        self.context_params.embeddings
    }

    /// Set whether to offload KQV ops to GPU
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default()
    ///     .with_offload_kqv(false);
    /// assert_eq!(params.offload_kqv(), false);
    /// ```
    #[must_use]
    pub fn with_offload_kqv(mut self, enabled: bool) -> Self {
        self.context_params.offload_kqv = enabled;
        self
    }

    /// Get whether KQV ops are offloaded to GPU
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// assert_eq!(params.offload_kqv(), true);
    /// ```
    #[must_use]
    pub fn offload_kqv(&self) -> bool {
        self.context_params.offload_kqv
    }

    /// Set whether to disable performance timings
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default().with_no_perf(true);
    /// assert!(params.no_perf());
    /// ```
    #[must_use]
    pub fn with_no_perf(mut self, no_perf: bool) -> Self {
        self.context_params.no_perf = no_perf;
        self
    }

    /// Get whether performance timings are disabled
    #[must_use]
    pub fn no_perf(&self) -> bool {
        self.context_params.no_perf
    }

    /// Set whether to offload ops to GPU
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default().with_op_offload(false);
    /// assert_eq!(params.op_offload(), false);
    /// ```
    #[must_use]
    pub fn with_op_offload(mut self, op_offload: bool) -> Self {
        self.context_params.op_offload = op_offload;
        self
    }

    /// Get whether ops are offloaded to GPU
    #[must_use]
    pub fn op_offload(&self) -> bool {
        self.context_params.op_offload
    }

    /// Set whether to use full sliding window attention
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default()
    ///     .with_swa_full(false);
    /// assert_eq!(params.swa_full(), false);
    /// ```
    #[must_use]
    pub fn with_swa_full(mut self, enabled: bool) -> Self {
        self.context_params.swa_full = enabled;
        self
    }

    /// Get whether full sliding window attention is enabled
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// assert_eq!(params.swa_full(), true);
    /// ```
    #[must_use]
    pub fn swa_full(&self) -> bool {
        self.context_params.swa_full
    }

    /// Set whether to use a unified KV cache buffer across input sequences
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default().with_kv_unified(true);
    /// assert!(params.kv_unified());
    /// ```
    #[must_use]
    pub fn with_kv_unified(mut self, kv_unified: bool) -> Self {
        self.context_params.kv_unified = kv_unified;
        self
    }

    /// Get whether a unified KV cache buffer is used across input sequences
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use koharu_llama::context::params::LlamaContextParams;
    /// let params = LlamaContextParams::default();
    /// let _ = params.kv_unified();
    /// ```
    #[must_use]
    pub fn kv_unified(&self) -> bool {
        self.context_params.kv_unified
    }
}
