//! Gemma 4's call syntax, `call:name{key:value,...}`, as its chat template's
//! `format_argument` writes it: strings between `<|"|>`, bare keys, and bare
//! numbers, booleans, and nulls.

use std::sync::Arc;

use rig_core::completion::ToolDefinition;
use serde_json::{Map, Value};

use super::Call;

const QUOTE: &str = "<|\"|>";

pub(super) fn parse(body: &str, _tools: &[Arc<ToolDefinition>]) -> Result<Call, String> {
    let body = body.trim();
    let call = body
        .strip_prefix("call:")
        .and_then(|call| call.split_once('{'))
        .filter(|(name, _)| !name.trim().is_empty());
    let Some((name, arguments)) = call else {
        return Err(format!("a Gemma 4 tool call names no tool: {body}"));
    };
    let written = format!("{{{arguments}");
    let mut reader = Reader(&written);
    let arguments = match reader.value() {
        Some(value) if value.is_object() && reader.0.trim().is_empty() => value.to_string(),
        _ => written,
    };
    Ok(Call {
        name: name.trim().to_owned(),
        arguments,
    })
}

/// Reads values off the front of what's left of a call's arguments.
struct Reader<'a>(&'a str);

impl Reader<'_> {
    fn value(&mut self) -> Option<Value> {
        self.0 = self.0.trim_start();
        if self.eat(QUOTE) {
            return self.quoted().map(Value::String);
        }
        if self.eat("{") {
            return self.object();
        }
        if self.eat("[") {
            return self.array();
        }
        let end = self.0.find([',', '}', ']']).unwrap_or(self.0.len());
        let literal = serde_json::from_str(self.0[..end].trim()).ok()?;
        self.0 = &self.0[end..];
        matches!(literal, Value::Null | Value::Bool(_) | Value::Number(_)).then_some(literal)
    }

    fn quoted(&mut self) -> Option<String> {
        let (text, rest) = self.0.split_once(QUOTE)?;
        self.0 = rest;
        Some(text.to_owned())
    }

    fn object(&mut self) -> Option<Value> {
        let mut object = Map::new();
        loop {
            self.0 = self.0.trim_start();
            if self.eat("}") {
                return Some(Value::Object(object));
            }
            let key = if self.eat(QUOTE) {
                self.quoted()?
            } else {
                let end = self.0.find(':')?;
                let key = self.0[..end].trim().to_owned();
                self.0 = &self.0[end..];
                key
            };
            if !self.eat(":") {
                return None;
            }
            object.insert(key, self.value()?);
            self.separator('}')?;
        }
    }

    fn array(&mut self) -> Option<Value> {
        let mut array = Vec::new();
        loop {
            self.0 = self.0.trim_start();
            if self.eat("]") {
                return Some(Value::Array(array));
            }
            array.push(self.value()?);
            self.separator(']')?;
        }
    }

    /// Takes a comma, or checks that `close` comes next.
    fn separator(&mut self, close: char) -> Option<()> {
        self.0 = self.0.trim_start();
        if self.eat(",") || self.0.starts_with(close) {
            Some(())
        } else {
            None
        }
    }

    fn eat(&mut self, text: &str) -> bool {
        match self.0.strip_prefix(text) {
            Some(rest) => {
                self.0 = rest;
                true
            }
            None => false,
        }
    }
}
