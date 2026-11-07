//! Each model family's native tool-call and reasoning markup, as the catalog's
//! chat templates write it, and the parser that reads it from a reply as the
//! reply streams.

mod function_tags;
mod gemma4;

use std::sync::Arc;

use rig_core::completion::ToolDefinition;

/// What a model's replies can hold besides their answer.
#[derive(Debug)]
pub(crate) struct Markup {
    /// The markers that open and close reasoning.
    pub(crate) reasoning: Option<(&'static str, &'static str)>,
    pub(crate) call: Option<CallMarkup>,
}

#[derive(Debug)]
pub(crate) struct CallMarkup {
    open: &'static str,
    close: &'static str,
    /// One call's body. An error means no tool can be named; arguments that
    /// don't parse stay as written, for rig to send the model back to.
    parse: fn(&str, &[Arc<ToolDefinition>]) -> Result<Call, String>,
}

pub(crate) static GEMMA4: Markup = Markup {
    reasoning: Some(GEMMA4_REASONING),
    call: Some(CallMarkup {
        open: "<|tool_call>",
        close: "<tool_call|>",
        parse: gemma4::parse,
    }),
};

/// Gemma 4 templates that render a tool result as a `tool` turn, a form
/// llama.cpp's `convert_tool_responses_gemma4` rewrites as outdated.
pub(crate) static GEMMA4_NO_TOOLS: Markup = Markup {
    reasoning: Some(GEMMA4_REASONING),
    call: None,
};

const GEMMA4_REASONING: (&str, &str) = ("<|channel>thought", "<channel|>");

/// Qwen's and LFM3's.
pub(crate) static FUNCTION_TAGS: Markup = Markup {
    reasoning: Some(("<think>", "</think>")),
    call: Some(CallMarkup {
        open: "<tool_call>",
        close: "</tool_call>",
        parse: function_tags::parse,
    }),
};

/// A tool call: its name, and arguments that are JSON unless the model wrote
/// them wrong.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Call {
    pub(crate) name: String,
    pub(crate) arguments: String,
}

/// One piece of a reply.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Item {
    Text(String),
    Reasoning(String),
    Call(Call),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Text,
    Reasoning,
    Call,
}

/// Reads a reply's text into [`Item`]s, holding back what could be the start
/// of a marker.
pub(crate) struct Parser {
    reasoning: Option<(&'static str, &'static str)>,
    call: Option<&'static CallMarkup>,
    tools: Vec<Arc<ToolDefinition>>,
    state: State,
    held: String,
    /// Whitespace right after a marker belongs to no item.
    skip_space: bool,
}

impl Parser {
    /// A parser that reads every reply as answer text.
    pub(crate) fn plain() -> Self {
        Self {
            reasoning: None,
            call: None,
            tools: Vec::new(),
            state: State::Text,
            held: String::new(),
            skip_space: false,
        }
    }

    /// A parser for the reply to `prompt`. Calls are only looked for when the
    /// request offered `tools`. The reply starts in reasoning when the
    /// template left a reasoning marker open at the prompt's end.
    pub(crate) fn new(
        markup: &'static Markup,
        tools: Vec<Arc<ToolDefinition>>,
        prompt: &str,
    ) -> Self {
        let reasoning = markup.reasoning;
        let open = reasoning.is_some_and(|(open, _)| prompt.trim_end().ends_with(open));
        Self {
            reasoning,
            call: markup.call.as_ref().filter(|_| !tools.is_empty()),
            tools,
            state: if open { State::Reasoning } else { State::Text },
            held: String::new(),
            skip_space: open,
        }
    }

    pub(crate) fn feed(&mut self, text: &str) -> Vec<Result<Item, String>> {
        self.held.push_str(text);
        let mut items = Vec::new();
        self.read(false, &mut items);
        items
    }

    /// What's left at the end of the reply.
    pub(crate) fn finish(&mut self) -> Vec<Result<Item, String>> {
        let mut items = Vec::new();
        self.read(true, &mut items);
        items
    }

    fn read(&mut self, end: bool, items: &mut Vec<Result<Item, String>>) {
        loop {
            let done = match self.state {
                State::Text => self.read_text(end, items),
                State::Reasoning => self.read_reasoning(end, items),
                State::Call => self.read_call(end, items),
            };
            if done {
                return;
            }
        }
    }

    /// Reads answer text up to the next opening marker; answers whether it
    /// needs more of the reply.
    fn read_text(&mut self, end: bool, items: &mut Vec<Result<Item, String>>) -> bool {
        let reasoning = self.reasoning.map(|(open, _)| (open, State::Reasoning));
        let call = self.call.map(|call| (call.open, State::Call));
        let markers = [reasoning, call];
        let next = markers
            .iter()
            .flatten()
            .filter_map(|&(marker, state)| Some((self.held.find(marker)?, marker, state)))
            .min_by_key(|&(at, ..)| at);
        match next {
            Some((at, marker, state)) => {
                self.emit(at, marker.len(), Item::Text, items);
                self.state = state;
                self.skip_space = true;
                false
            }
            None => {
                let held = if end {
                    0
                } else {
                    let markers = markers.iter().flatten().map(|&(marker, _)| marker);
                    partial(&self.held, markers)
                };
                self.emit(self.held.len() - held, 0, Item::Text, items);
                true
            }
        }
    }

    fn read_reasoning(&mut self, end: bool, items: &mut Vec<Result<Item, String>>) -> bool {
        let Some((_, close)) = self.reasoning else {
            self.state = State::Text;
            return false;
        };
        match self.held.find(close) {
            Some(at) => {
                self.emit(at, close.len(), Item::Reasoning, items);
                self.state = State::Text;
                self.skip_space = true;
                false
            }
            None => {
                let held = if end {
                    0
                } else {
                    partial(&self.held, [close].into_iter())
                };
                self.emit(self.held.len() - held, 0, Item::Reasoning, items);
                true
            }
        }
    }

    fn read_call(&mut self, end: bool, items: &mut Vec<Result<Item, String>>) -> bool {
        let Some(call) = self.call else {
            self.state = State::Text;
            return false;
        };
        match self.held.find(call.close) {
            Some(at) => {
                items.push((call.parse)(&self.held[..at], &self.tools).map(Item::Call));
                self.held.drain(..at + call.close.len());
                self.state = State::Text;
                self.skip_space = true;
                false
            }
            None if end => {
                if !self.held.trim().is_empty() {
                    items.push((call.parse)(&self.held, &self.tools).map(Item::Call));
                }
                self.held.clear();
                true
            }
            None => true,
        }
    }

    /// Sends the first `length` bytes as an item, without the whitespace a
    /// marker left, and takes them and a marker of `marker` bytes off.
    fn emit(
        &mut self,
        length: usize,
        marker: usize,
        item: fn(String) -> Item,
        items: &mut Vec<Result<Item, String>>,
    ) {
        let text = &self.held[..length];
        let text = if self.skip_space {
            text.trim_start()
        } else {
            text
        };
        if !text.is_empty() {
            items.push(Ok(item(text.to_owned())));
            self.skip_space = false;
        }
        self.held.drain(..length + marker);
    }
}

/// How many of `text`'s last bytes could be the start of one of `markers`.
fn partial<'a>(text: &str, markers: impl Iterator<Item = &'a str>) -> usize {
    markers
        .filter_map(|marker| {
            (1..marker.len())
                .rev()
                .find(|&length| text.ends_with(&marker[..length]))
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rig_core::message::ToolName;
    use serde_json::{Value, json};

    use super::*;

    fn tools() -> Vec<Arc<ToolDefinition>> {
        vec![Arc::new(ToolDefinition {
            name: ToolName::new("get_page").unwrap(),
            description: String::new(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "page": {"type": "integer"},
                    "title": {"type": "string"},
                    "number": {"type": "string"},
                },
            }),
            annotations: Default::default(),
        })]
    }

    fn all(parser: &mut Parser, pieces: &[&str]) -> Vec<Item> {
        let mut items: Vec<Item> = Vec::new();
        let mut results = Vec::new();
        for piece in pieces {
            results.extend(parser.feed(piece));
        }
        results.extend(parser.finish());
        for result in results {
            let item = result.unwrap();
            // Streaming splits text and reasoning at arbitrary points.
            match (items.last_mut(), item) {
                (Some(Item::Text(held)), Item::Text(more))
                | (Some(Item::Reasoning(held)), Item::Reasoning(more)) => held.push_str(&more),
                (_, item) => items.push(item),
            }
        }
        items
    }

    /// The same items whether the reply arrives whole or cut anywhere.
    fn parses(markup: &'static Markup, prompt: &str, reply: &str, expected: &[Item]) {
        let whole = all(&mut Parser::new(markup, tools(), prompt), &[reply]);
        assert_eq!(whole, expected, "whole");
        for (at, _) in reply.char_indices().skip(1) {
            let cut = all(
                &mut Parser::new(markup, tools(), prompt),
                &[&reply[..at], &reply[at..]],
            );
            assert_eq!(cut, expected, "cut at byte {at}");
        }
    }

    fn call(name: &str, arguments: Value) -> Item {
        Item::Call(Call {
            name: name.to_owned(),
            arguments: arguments.to_string(),
        })
    }

    fn arguments(item: &Item) -> Value {
        let Item::Call(call) = item else {
            panic!("a call");
        };
        serde_json::from_str(&call.arguments).unwrap()
    }

    #[test]
    fn gemma4() {
        let reply = indoc! {r#"
            <|channel>thought
            I should read it.
            <channel|><|tool_call>call:get_page{flags:[<|"|>a<|"|>,1],on:true,page:3,title:<|"|>一, {two}<|"|>,nested:{x:null}}<tool_call|>"#
        };
        let items = all(
            &mut Parser::new(&GEMMA4, tools(), "<|turn>model\n"),
            &[reply],
        );
        assert_eq!(items[0], Item::Reasoning("I should read it.\n".to_owned()));
        assert_eq!(
            arguments(&items[1]),
            json!({
                "flags": ["a", 1],
                "on": true,
                "page": 3,
                "title": "一, {two}",
                "nested": {"x": null},
            })
        );
        parses(&GEMMA4, "<|turn>model\n", reply, &items);
    }

    #[test]
    fn gemma4_starts_in_reasoning_after_a_tool_result() {
        parses(
            &GEMMA4,
            "<tool_response|><|channel>thought\n",
            "Done.\n<channel|>The answer.",
            &[
                Item::Reasoning("Done.\n".to_owned()),
                Item::Text("The answer.".to_owned()),
            ],
        );
    }

    #[test]
    fn gemma4_drops_an_empty_thought() {
        parses(
            &GEMMA4,
            "<|turn>model\n",
            "<|channel>thought\n<channel|>The answer.",
            &[Item::Text("The answer.".to_owned())],
        );
    }

    #[test]
    fn gemma4_arguments_that_do_not_parse_stay_as_written() {
        let reply = "<|tool_call>call:get_page{page:three}<tool_call|>";
        parses(
            &GEMMA4,
            "",
            reply,
            &[Item::Call(Call {
                name: "get_page".to_owned(),
                arguments: "{page:three}".to_owned(),
            })],
        );
    }

    #[test]
    fn function_tags() {
        let reply = indoc! {"
            <think>
            I should read it.
            </think>

            Reading.

            <tool_call>
            <function=get_page>
            <parameter=page>
            3
            </parameter>
            <parameter=title>
            3
            </parameter>
            <parameter=number>
            </parameter>
            </function>
            </tool_call>
            <tool_call>
            <function=get_page>
            </function>
            </tool_call>"
        };
        let items = all(
            &mut Parser::new(&FUNCTION_TAGS, tools(), "<|im_start|>assistant\n"),
            &[reply],
        );
        assert_eq!(items[0], Item::Reasoning("I should read it.\n".to_owned()));
        assert_eq!(items[1], Item::Text("Reading.\n\n".to_owned()));
        // A string stays text even when it reads as a number.
        assert_eq!(
            arguments(&items[2]),
            json!({"page": 3, "title": "3", "number": ""})
        );
        assert_eq!(items[3], call("get_page", json!({})));
        parses(&FUNCTION_TAGS, "<|im_start|>assistant\n", reply, &items);
    }

    #[test]
    fn function_tags_start_in_reasoning_when_the_prompt_opens_it() {
        for prompt in ["assistant\n<think>\n", "assistant\n<think>"] {
            parses(
                &FUNCTION_TAGS,
                prompt,
                "Thinking.</think>The answer.",
                &[
                    Item::Reasoning("Thinking.".to_owned()),
                    Item::Text("The answer.".to_owned()),
                ],
            );
        }
        // Reasoning turned off leaves the prompt ending in the closing marker.
        parses(
            &FUNCTION_TAGS,
            "assistant\n<think></think>",
            "The answer.",
            &[Item::Text("The answer.".to_owned())],
        );
    }

    #[test]
    fn no_tools_means_no_calls() {
        let reply = "<tool_call>not a call</tool_call>";
        let items = all(&mut Parser::new(&FUNCTION_TAGS, Vec::new(), ""), &[reply]);
        assert_eq!(items, [Item::Text(reply.to_owned())]);
    }

    #[test]
    fn a_plain_parser_reads_markers_as_text() {
        let reply = "{\"text\": \"<think>a</think><tool_call>\"}";
        assert_eq!(
            all(&mut Parser::plain(), &[reply]),
            [Item::Text(reply.to_owned())]
        );
    }

    #[test]
    fn a_call_cut_short_still_reaches_rig() {
        let items = all(
            &mut Parser::new(&FUNCTION_TAGS, tools(), ""),
            &["<tool_call><function=get_page><parameter=page>3"],
        );
        assert_eq!(
            items,
            [Item::Call(Call {
                name: "get_page".to_owned(),
                arguments: "<parameter=page>3".to_owned(),
            })]
        );
    }

    #[test]
    fn a_call_with_no_tool_named_fails() {
        let mut parser = Parser::new(&FUNCTION_TAGS, tools(), "");
        let results = parser.feed("<tool_call>nothing</tool_call>");
        assert!(results[0].is_err());
    }
}
