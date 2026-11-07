//! A GGUF's chat template, rendered with minijinja as llama.cpp renders it
//! with its own Jinja engine.

use anyhow::{Context, Result};
use koharu_llama::model::LlamaModel;
use minijinja::{
    Environment, Error, ErrorKind, context,
    value::{Serde, Value as Jinja},
};
use serde_json::Value;

const NAME: &str = "chat";

#[derive(Debug)]
pub(super) struct Template {
    environment: Environment<'static>,
    bos: String,
    eos: String,
    /// Whether tokenizing the prompt adds the BOS token itself.
    adds_bos: bool,
}

/// What a prompt is rendered from.
pub(super) struct Rendering<'a> {
    pub(super) messages: &'a [Value],
    pub(super) tools: &'a [Value],
    pub(super) thinking: bool,
    /// What the template gets as `reasoning_effort`, which only some read.
    pub(super) effort: Option<&'a str>,
}

impl Template {
    pub(super) fn new(model: &LlamaModel) -> Result<Self> {
        let source = model
            .chat_template(None)
            .context("the GGUF has no chat template")?
            .to_string()
            .context("the GGUF's chat template isn't UTF-8")?;
        let vocab = model.vocab();
        let text = |token| String::from_utf8(vocab.token_to_piece(token, true, None));
        Ok(Self {
            environment: environment(source)?,
            bos: text(vocab.bos()).context("the BOS token isn't UTF-8")?,
            eos: text(vocab.eos()).context("the EOS token isn't UTF-8")?,
            adds_bos: vocab.should_add_bos(),
        })
    }

    /// The prompt, ready for the model's answer.
    pub(super) fn render(&self, rendering: &Rendering) -> Result<String> {
        let prompt = self
            .environment
            .get_template(NAME)?
            .render(context! {
                messages => Jinja::from(Serde(rendering.messages)),
                tools => Jinja::from(Serde(rendering.tools)),
                bos_token => self.bos.as_str(),
                eos_token => self.eos.as_str(),
                add_generation_prompt => true,
                enable_thinking => rendering.thinking,
                reasoning_effort => rendering.effort.map_or(Jinja::UNDEFINED, Jinja::from),
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

#[cfg(test)]
impl Template {
    pub(super) fn from_source(source: &str) -> Self {
        Self {
            environment: environment(source.to_owned()).unwrap(),
            bos: "<bos>".to_owned(),
            eos: "<eos>".to_owned(),
            adds_bos: true,
        }
    }
}

fn environment(source: String) -> Result<Environment<'static>> {
    // Transformers' tags, which llama.cpp's Jinja parser ignores and minijinja
    // refuses; another spelling fails to compile below.
    let source = source
        .replace("{%- generation -%}", "")
        .replace("{%- endgeneration -%}", "");
    let mut environment = Environment::new();
    minijinja_contrib::add_to_environment(&mut environment);
    environment.set_unknown_method_callback(minijinja_contrib::pycompat::unknown_method_callback);
    environment.add_function(
        "raise_exception",
        |message: String| -> Result<String, Error> {
            Err(Error::new(ErrorKind::InvalidOperation, message))
        },
    );
    environment
        .add_template_owned(NAME, source)
        .context("couldn't compile the GGUF's chat template")?;
    Ok(environment)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn transformers_generation_tags_render_their_contents() {
        let template = Template::from_source(
            "{%- for message in messages -%}{%- generation -%}{{ message.content }}{%- endgeneration -%}{%- endfor -%}",
        );
        let prompt = template
            .render(&Rendering {
                messages: &[json!({"role": "assistant", "content": "kept"})],
                tools: &[],
                thinking: false,
                effort: None,
            })
            .unwrap();
        assert_eq!(prompt, "kept");
    }

    /// Qwen 3.8's template raises on an effort it doesn't know and defaults
    /// an undefined one, so an unset effort must stay undefined.
    #[test]
    fn an_unset_effort_is_undefined() {
        let template = Template::from_source("{{ reasoning_effort | default('xhigh') }}");
        let rendering = |effort| Rendering {
            messages: &[],
            tools: &[],
            thinking: true,
            effort,
        };
        assert_eq!(template.render(&rendering(None)).unwrap(), "xhigh");
        assert_eq!(template.render(&rendering(Some("low"))).unwrap(), "low");
    }

    #[test]
    fn a_templates_own_bos_is_dropped() {
        let template = Template::from_source("{{ bos_token }}Hi");
        let prompt = template
            .render(&Rendering {
                messages: &[],
                tools: &[],
                thinking: false,
                effort: None,
            })
            .unwrap();
        assert_eq!(prompt, "Hi");
    }
}
