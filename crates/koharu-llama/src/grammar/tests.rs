use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use super::*;

#[test]
fn check_parse() {
    let dir = Path::new("src/grammar");
    let files = std::fs::read_dir(dir)
        .expect("Failed to read grammar directory")
        .filter_map(Result::ok)
        .map(|os_str| os_str.path())
        .filter(|p| p.is_file())
        .filter(|f| f.extension().unwrap_or_default() == "gbnf")
        .map(File::open)
        .collect::<Vec<_>>();
    assert!(
        !files.is_empty(),
        "No grammar files found in {}",
        dir.canonicalize().unwrap().display()
    );
    for file in files {
        let reader = BufReader::new(file.unwrap());
        let file = std::io::read_to_string(reader).unwrap();
        LlamaGrammar::from_str(&file).unwrap();
    }
}

#[test]
fn check_parse_simple() {
    let parse_state = ParseState::from_str(r#"root ::= "cat""#).unwrap();
    assert_eq!(
        ParseState {
            symbol_ids: BTreeMap::from([("root".to_string(), 0),]),
            rules: vec![vec![
                llama_grammar_element {
                    type_: koharu_llama_sys::LLAMA_GRETYPE_CHAR,
                    value: 'c' as u32,
                },
                llama_grammar_element {
                    type_: koharu_llama_sys::LLAMA_GRETYPE_CHAR,
                    value: 'a' as u32,
                },
                llama_grammar_element {
                    type_: koharu_llama_sys::LLAMA_GRETYPE_CHAR,
                    value: 't' as u32,
                },
                llama_grammar_element {
                    type_: koharu_llama_sys::LLAMA_GRETYPE_END,
                    value: 0,
                }
            ]],
        },
        parse_state
    );
}

#[test]
fn check_parse_char_range() {
    let parse_state = ParseState::from_str(r#"root ::= [a-zA-Z]"#).unwrap();
    assert_eq!(
        ParseState {
            symbol_ids: BTreeMap::from([("root".to_string(), 0),]),
            rules: vec![vec![
                llama_grammar_element {
                    type_: koharu_llama_sys::LLAMA_GRETYPE_CHAR,
                    value: 'a' as u32
                },
                llama_grammar_element {
                    type_: koharu_llama_sys::LLAMA_GRETYPE_CHAR_RNG_UPPER,
                    value: 'z' as u32
                },
                llama_grammar_element {
                    type_: koharu_llama_sys::LLAMA_GRETYPE_CHAR_ALT,
                    value: 'A' as u32
                },
                llama_grammar_element {
                    type_: koharu_llama_sys::LLAMA_GRETYPE_CHAR_RNG_UPPER,
                    value: 'Z' as u32
                },
                llama_grammar_element {
                    type_: koharu_llama_sys::LLAMA_GRETYPE_END,
                    value: 0
                }
            ]]
        },
        parse_state
    );
}
