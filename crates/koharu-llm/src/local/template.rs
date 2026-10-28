//! A GGUF's chat template, rendered with minijinja as llama.cpp renders it
//! with its own Jinja engine.

use anyhow::{Context, Result};
use llama_cpp_2::model::LlamaModel;
use minijinja::{Environment, Error, ErrorKind, context};

const NAME: &str = "chat";

#[derive(Debug)]
pub(super) struct Template {
    environment: Environment<'static>,
    bos: String,
    eos: String,
    /// Whether tokenizing the prompt adds the BOS token itself.
    adds_bos: bool,
}

impl Template {
    pub(super) fn new(model: &LlamaModel) -> Result<Self> {
        let source = model
            .chat_template(None)
            .context("the GGUF has no chat template")?
            .to_string()
            .context("the GGUF's chat template isn't UTF-8")?;
        let mut environment = Environment::new();
        minijinja_contrib::add_to_environment(&mut environment);
        environment
            .set_unknown_method_callback(minijinja_contrib::pycompat::unknown_method_callback);
        environment.add_function(
            "raise_exception",
            |message: String| -> Result<String, Error> {
                Err(Error::new(ErrorKind::InvalidOperation, message))
            },
        );
        environment
            .add_template_owned(NAME, source)
            .context("couldn't compile the GGUF's chat template")?;
        let vocab = model.vocab();
        let text = |token| String::from_utf8(vocab.token_to_piece(token, true, None));
        Ok(Self {
            environment,
            bos: text(vocab.bos()).context("the BOS token isn't UTF-8")?,
            eos: text(vocab.eos()).context("the EOS token isn't UTF-8")?,
            adds_bos: vocab.should_add_bos(),
        })
    }

    /// The prompt for `messages`, `(role, content)` pairs, ready for the
    /// model's answer.
    pub(super) fn render(&self, messages: &[(&str, String)], thinking: bool) -> Result<String> {
        let messages = messages
            .iter()
            .map(|&(role, ref content)| context! { role, content })
            .collect::<Vec<_>>();
        let prompt = self
            .environment
            .get_template(NAME)?
            .render(context! {
                messages,
                bos_token => self.bos.as_str(),
                eos_token => self.eos.as_str(),
                add_generation_prompt => true,
                enable_thinking => thinking,
            })
            .context("couldn't render the GGUF's chat template")?;
        // Tokenizing adds the BOS token, so a template's own is dropped, as
        // llama.cpp's common_chat_templates_apply drops it.
        Ok(match prompt.strip_prefix(self.bos.as_str()) {
            Some(rest) if self.adds_bos && !self.bos.is_empty() => rest.to_owned(),
            _ => prompt,
        })
    }
}
