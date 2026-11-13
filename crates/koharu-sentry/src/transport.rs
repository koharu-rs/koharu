//! Not upstream: Sentry's default transport, finishing what this crate
//! captures. An error event gets the window's image, and an interface event
//! gets back the source maps' debug images it was parsed without.

use std::{sync::Arc, time::Duration};

use sentry::{
    Envelope, Level, TransportFactory, TransportOptions, transports::DefaultTransportFactory,
};

use crate::{browser, screenshot};

/// Sentry's default transport, adding the window's image to each error
/// event and restoring interface events' source-map debug images. Set it as
/// the client's transport.
#[derive(Debug, Default)]
pub struct Transport;

impl TransportFactory for Transport {
    fn create_transport_with_options(
        &self,
        options: TransportOptions,
    ) -> Arc<dyn sentry::Transport> {
        Arc::new(Finishing(
            DefaultTransportFactory.create_transport_with_options(options),
        ))
    }
}

struct Finishing(Arc<dyn sentry::Transport>);

impl sentry::Transport for Finishing {
    fn send_envelope(&self, mut envelope: Envelope) {
        if envelope
            .event()
            .is_some_and(|event| event.level >= Level::Error)
            && let Some(image) = screenshot::taken()
        {
            envelope.add_item(image);
        }
        self.0.send_envelope(browser::restore(envelope));
    }

    fn flush(&self, timeout: Duration) -> bool {
        self.0.flush(timeout)
    }

    fn shutdown(&self, timeout: Duration) -> bool {
        self.0.shutdown(timeout)
    }
}
