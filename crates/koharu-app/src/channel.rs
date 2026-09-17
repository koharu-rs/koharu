use std::{marker::PhantomData, sync::Arc};

use serde::Serialize;
use serde_json::Value;
use specta::{Type, Types, datatype::DataType};

/// Project-owned event sink used by subscribe/login/run_agent over WebSocket.
pub struct Channel<T> {
    send: Arc<dyn Fn(Value) -> bool + Send + Sync>,
    _ty: PhantomData<fn() -> T>,
}

impl<T> Clone for Channel<T> {
    fn clone(&self) -> Self {
        Self {
            send: Arc::clone(&self.send),
            _ty: PhantomData,
        }
    }
}

impl<T: Serialize> Channel<T> {
    pub fn from_sink(send: impl Fn(Value) -> bool + Send + Sync + 'static) -> Self {
        Self {
            send: Arc::new(send),
            _ty: PhantomData,
        }
    }

    pub fn send(&self, value: T) -> Result<(), ()> {
        let value = serde_json::to_value(&value).map_err(|_| ())?;
        if (self.send)(value) { Ok(()) } else { Err(()) }
    }
}

impl<T: Type> Type for Channel<T> {
    fn definition(types: &mut Types) -> DataType {
        T::definition(types)
    }
}
