//! Embeddings.

use std::borrow::Borrow;

use koharu_torch::{Result, Tensor};

#[derive(Debug, Clone, Copy)]
pub struct EmbeddingConfig {
    pub sparse: bool,
    pub scale_grad_by_freq: bool,
    pub padding_idx: i64,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        EmbeddingConfig {
            sparse: false,
            scale_grad_by_freq: false,
            padding_idx: -1,
        }
    }
}

#[derive(Debug)]
pub struct Embedding {
    pub ws: Tensor,
    config: EmbeddingConfig,
}

pub fn embedding<'a, T: Borrow<super::Path<'a>>>(
    vs: T,
    num_embeddings: i64,
    embedding_dim: i64,
    config: EmbeddingConfig,
) -> Result<Embedding> {
    Ok(Embedding {
        ws: vs
            .borrow()
            .var("weight", &[num_embeddings, embedding_dim])?,
        config,
    })
}

impl super::Module for Embedding {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        Tensor::f_embedding(
            &self.ws,
            xs,
            self.config.padding_idx,
            self.config.scale_grad_by_freq,
            self.config.sparse,
        )
    }
}
