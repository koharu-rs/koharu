use super::LogOptions;
use tracing_core::{callsite, field, identify_callsite, Interest, Kind, Metadata};

static FIELD_NAMES: &[&str] = &["message", "module"];

struct OverridableFields {
    message: tracing::field::Field,
    target: tracing::field::Field,
}

macro_rules! log_cs {
    ($level:expr, $cs:ident, $meta:ident, $fields:ident, $ty:ident) => {
        struct $ty;
        static $cs: $ty = $ty;
        static $meta: Metadata<'static> = Metadata::new(
            "log event",
            "llama-cpp-2",
            $level,
            ::core::option::Option::None,
            ::core::option::Option::None,
            ::core::option::Option::None,
            field::FieldSet::new(FIELD_NAMES, identify_callsite!(&$cs)),
            Kind::EVENT,
        );
        static $fields: std::sync::LazyLock<OverridableFields> = std::sync::LazyLock::new(|| {
            let fields = $meta.fields();
            OverridableFields {
                message: fields.field("message").unwrap(),
                target: fields.field("module").unwrap(),
            }
        });

        impl callsite::Callsite for $ty {
            fn set_interest(&self, _: Interest) {}
            fn metadata(&self) -> &'static Metadata<'static> {
                &$meta
            }
        }
    };
}
log_cs!(
    tracing_core::Level::DEBUG,
    DEBUG_CS,
    DEBUG_META,
    DEBUG_FIELDS,
    DebugCallsite
);
log_cs!(
    tracing_core::Level::INFO,
    INFO_CS,
    INFO_META,
    INFO_FIELDS,
    InfoCallsite
);
log_cs!(
    tracing_core::Level::WARN,
    WARN_CS,
    WARN_META,
    WARN_FIELDS,
    WarnCallsite
);
log_cs!(
    tracing_core::Level::ERROR,
    ERROR_CS,
    ERROR_META,
    ERROR_FIELDS,
    ErrorCallsite
);

#[derive(Clone, Copy)]
pub(super) enum Module {
    GGML,
    LlamaCpp,
    /// Multimodal
    Mtmd,
}

impl Module {
    const fn name(&self) -> &'static str {
        match self {
            Module::GGML => "ggml",
            Module::LlamaCpp => "llama.cpp",
            Module::Mtmd => "mtmd",
        }
    }
}

fn meta_for_level(
    level: koharu_llama_sys::ggml_log_level,
) -> (&'static Metadata<'static>, &'static OverridableFields) {
    match level {
        koharu_llama_sys::GGML_LOG_LEVEL_DEBUG => (&DEBUG_META, &DEBUG_FIELDS),
        koharu_llama_sys::GGML_LOG_LEVEL_INFO => (&INFO_META, &INFO_FIELDS),
        koharu_llama_sys::GGML_LOG_LEVEL_WARN => (&WARN_META, &WARN_FIELDS),
        koharu_llama_sys::GGML_LOG_LEVEL_ERROR => (&ERROR_META, &ERROR_FIELDS),
        _ => {
            unreachable!("Illegal log level to be called here")
        }
    }
}

pub(super) struct State {
    pub(super) options: LogOptions,
    module: Module,
    buffered: std::sync::Mutex<Option<(koharu_llama_sys::ggml_log_level, String)>>,
    previous_level: std::sync::atomic::AtomicI32,
    is_buffering: std::sync::atomic::AtomicBool,
}

impl State {
    pub(super) fn new(module: Module, options: LogOptions) -> Self {
        Self {
            options,
            module,
            buffered: Default::default(),
            previous_level: Default::default(),
            is_buffering: Default::default(),
        }
    }

    fn generate_log(target: Module, level: koharu_llama_sys::ggml_log_level, text: &str) {
        // Annoying but tracing requires that the provided target name is a string literal and
        // even &'static str isn't enough so we have to duplicate the generation AND we can't even
        // extract the interrior module within llama.cpp/ggml to be able to propagate it forward.
        // This happens because the target is part of a static variable injected by the macro that's
        // initialized with said target.

        let (module, text) = text
            .char_indices()
            .take_while(|(_, c)| c.is_ascii_lowercase() || *c == '_')
            .last()
            .and_then(|(pos, _)| {
                let next_two = text.get(pos + 1..pos + 3);
                if next_two == Some(": ") {
                    let (sub_module, text) = text.split_at(pos + 1);
                    let text = text.split_at(2).1;
                    Some((Some(format!("{}::{sub_module}", target.name())), text))
                } else {
                    None
                }
            })
            .unwrap_or((None, text));

        let (meta, fields) = meta_for_level(level);

        tracing::dispatcher::get_default(|dispatcher| {
            dispatcher.event(&tracing::Event::new(
                meta,
                &meta.fields().value_set(&[
                    (&fields.message, Some(&text as &dyn tracing::field::Value)),
                    (
                        &fields.target,
                        module.as_ref().map(|s| s as &dyn tracing::field::Value),
                    ),
                ]),
            ));
        });
    }

    /// Append more text to the previously buffered log. The text may or may not end with a newline.
    pub(super) fn cont_buffered_log(&self, text: &str) {
        let mut lock = self.buffered.lock().unwrap();

        if let Some((previous_log_level, mut buffer)) = lock.take() {
            buffer.push_str(text);
            if buffer.ends_with('\n') {
                self.is_buffering
                    .store(false, std::sync::atomic::Ordering::Release);
                Self::generate_log(self.module, previous_log_level, buffer.as_str());
            } else {
                *lock = Some((previous_log_level, buffer));
            }
        } else {
            let level = self
                .previous_level
                .load(std::sync::atomic::Ordering::Acquire)
                as koharu_llama_sys::ggml_log_level;
            tracing::warn!(
                inferred_level = level,
                text = text,
                origin = "crate",
                "llma.cpp sent out a CONT log without any previously buffered message"
            );
            *lock = Some((level, text.to_string()));
        }
    }

    /// Start buffering a message. Not the CONT log level and text is missing a newline.
    pub(super) fn buffer_non_cont(&self, level: koharu_llama_sys::ggml_log_level, text: &str) {
        debug_assert!(!text.ends_with('\n'));
        debug_assert_ne!(level, koharu_llama_sys::GGML_LOG_LEVEL_CONT);

        if let Some((previous_log_level, buffer)) = self
            .buffered
            .lock()
            .unwrap()
            .replace((level, text.to_string()))
        {
            tracing::warn!(
                level = previous_log_level,
                text = &buffer,
                origin = "crate",
                "Message buffered unnnecessarily due to missing newline and not followed by a CONT"
            );
            Self::generate_log(self.module, previous_log_level, buffer.as_str())
        }

        self.is_buffering
            .store(true, std::sync::atomic::Ordering::Release);
        self.previous_level
            .store(level as i32, std::sync::atomic::Ordering::Release);
    }

    // Emit a normal unbuffered log message (not the CONT log level and the text ends with a newline).
    pub(super) fn emit_non_cont_line(&self, level: koharu_llama_sys::ggml_log_level, text: &str) {
        debug_assert!(text.ends_with('\n'));
        debug_assert_ne!(level, koharu_llama_sys::GGML_LOG_LEVEL_CONT);

        if self
            .is_buffering
            .swap(false, std::sync::atomic::Ordering::Acquire)
        {
            if let Some((buf_level, buf_text)) = self.buffered.lock().unwrap().take() {
                // This warning indicates a bug within llama.cpp
                tracing::warn!(level = buf_level, text = buf_text, origin = "crate", "llama.cpp message buffered spuriously due to missing \\n and being followed by a non-CONT message!");
                Self::generate_log(self.module, buf_level, buf_text.as_str());
            }
        }

        self.previous_level
            .store(level as i32, std::sync::atomic::Ordering::Release);

        let (text, newline) = text.split_at(text.len() - 1);
        debug_assert_eq!(newline, "\n");

        match level {
            koharu_llama_sys::GGML_LOG_LEVEL_NONE => {
                // TODO: Support logging this to stdout directly via options?
                tracing::info!(no_log_level = true, text);
            }
            koharu_llama_sys::GGML_LOG_LEVEL_DEBUG
            | koharu_llama_sys::GGML_LOG_LEVEL_INFO
            | koharu_llama_sys::GGML_LOG_LEVEL_WARN
            | koharu_llama_sys::GGML_LOG_LEVEL_ERROR => {
                Self::generate_log(self.module, level, text)
            }
            koharu_llama_sys::GGML_LOG_LEVEL_CONT => unreachable!(),
            _ => {
                tracing::warn!(
                    level = level,
                    text = text,
                    origin = "crate",
                    "Unknown llama.cpp log level"
                )
            }
        }
    }

    pub(super) fn update_previous_level_for_disabled_log(
        &self,
        level: koharu_llama_sys::ggml_log_level,
    ) {
        if level != koharu_llama_sys::GGML_LOG_LEVEL_CONT {
            self.previous_level
                .store(level as i32, std::sync::atomic::Ordering::Release);
        }
    }

    /// Checks whether the given log level is enabled by the current tracing subscriber.
    pub(super) fn is_enabled_for_level(&self, level: koharu_llama_sys::ggml_log_level) -> bool {
        // CONT logs do not need to check if they are enabled.
        let level = if level == koharu_llama_sys::GGML_LOG_LEVEL_CONT {
            self.previous_level
                .load(std::sync::atomic::Ordering::Relaxed)
                as koharu_llama_sys::ggml_log_level
        } else {
            level
        };
        let (meta, _) = meta_for_level(level);
        tracing::dispatcher::get_default(|dispatcher| dispatcher.enabled(meta))
    }
}

#[cfg(test)]
mod tests {
    use crate::logs_to_trace;
    use std::sync::{Arc, Mutex};
    use tracing::subscriber::DefaultGuard;
    use tracing_subscriber::util::SubscriberInitExt;

    use super::*;

    struct Logger {
        #[allow(unused)]
        guard: DefaultGuard,
        logs: Arc<Mutex<Vec<String>>>,
    }

    #[derive(Clone)]
    struct VecWriter(Arc<Mutex<Vec<String>>>);

    impl std::io::Write for VecWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let log_line = String::from_utf8(buf.to_vec()).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid UTF-8")
            })?;
            self.0.lock().unwrap().push(log_line);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn create_logger(max_level: tracing::Level) -> Logger {
        let logs = Arc::new(Mutex::new(vec![]));
        let writer = VecWriter(logs.clone());

        Logger {
            guard: tracing_subscriber::fmt()
                .with_max_level(max_level)
                .with_ansi(false)
                .without_time()
                .with_file(false)
                .with_line_number(false)
                .with_level(false)
                .with_target(false)
                .with_writer(move || writer.clone())
                .finish()
                .set_default(),
            logs,
        }
    }

    #[test]
    fn cont_disabled_log() {
        let logger = create_logger(tracing::Level::INFO);
        let mut log_state = Box::new(State::new(Module::LlamaCpp, LogOptions::default()));
        let log_ptr = log_state.as_mut() as *mut State as *mut std::os::raw::c_void;

        logs_to_trace(
            koharu_llama_sys::GGML_LOG_LEVEL_DEBUG,
            c"Hello ".as_ptr(),
            log_ptr,
        );
        logs_to_trace(
            koharu_llama_sys::GGML_LOG_LEVEL_CONT,
            c"world\n".as_ptr(),
            log_ptr,
        );

        assert!(logger.logs.lock().unwrap().is_empty());

        logs_to_trace(
            koharu_llama_sys::GGML_LOG_LEVEL_DEBUG,
            c"Hello ".as_ptr(),
            log_ptr,
        );
        logs_to_trace(
            koharu_llama_sys::GGML_LOG_LEVEL_CONT,
            c"world".as_ptr(),
            log_ptr,
        );
        logs_to_trace(
            koharu_llama_sys::GGML_LOG_LEVEL_CONT,
            c"\n".as_ptr(),
            log_ptr,
        );
    }

    #[test]
    fn cont_enabled_log() {
        let logger = create_logger(tracing::Level::INFO);
        let mut log_state = Box::new(State::new(Module::LlamaCpp, LogOptions::default()));
        let log_ptr = log_state.as_mut() as *mut State as *mut std::os::raw::c_void;

        logs_to_trace(
            koharu_llama_sys::GGML_LOG_LEVEL_INFO,
            c"Hello ".as_ptr(),
            log_ptr,
        );
        logs_to_trace(
            koharu_llama_sys::GGML_LOG_LEVEL_CONT,
            c"world\n".as_ptr(),
            log_ptr,
        );

        // Not sure where the extra \n comes from.
        assert_eq!(*logger.logs.lock().unwrap(), vec!["Hello world\n\n"]);
    }
}
