use std::future::Future;

use anyhow::Result;

pub(crate) struct ModelCell<M> {
    model: tokio::sync::Mutex<Option<M>>,
}

impl<M> ModelCell<M> {
    pub(crate) fn new() -> Self {
        Self {
            model: tokio::sync::Mutex::new(None),
        }
    }

    pub(crate) async fn ensure<F, Fut>(&self, load: F) -> Result<()>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<M>>,
    {
        let mut model = self.model.lock().await;
        if model.is_none() {
            *model = Some(load().await?);
        }
        Ok(())
    }

    pub(crate) async fn lock(&self) -> tokio::sync::MutexGuard<'_, Option<M>> {
        self.model.lock().await
    }

    pub(crate) fn unload(&self) -> bool {
        self.model
            .try_lock()
            .map(|mut model| model.take().is_some())
            .unwrap_or(false)
    }
}

impl<M> Default for ModelCell<M> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loads_once_until_unloaded() {
        let cell = ModelCell::new();
        cell.ensure(|| async { Ok(1_u32) }).await.unwrap();
        cell.ensure(|| async { Ok(2_u32) }).await.unwrap();
        assert_eq!(*cell.lock().await, Some(1));
        assert!(cell.unload());
        cell.ensure(|| async { Ok(3_u32) }).await.unwrap();
        assert_eq!(*cell.lock().await, Some(3));
    }
}
