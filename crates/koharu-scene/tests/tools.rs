//! The tools generated for agents and MCP clients, and the answers they get.

mod common;
use common::*;

#[test]
fn every_schema_reference_resolves() {
    for tool in tools() {
        let schema = tool.input_schema.to_string();
        for part in schema.split("\"#/$defs/").skip(1) {
            let name = &part[..part.find('"').unwrap()];
            assert!(
                tool.input_schema["$defs"].get(name).is_some(),
                "{}: missing {name}",
                tool.name
            );
        }
    }
}

#[test]
fn tools_cover_what_agents_send() {
    let names: Vec<&str> = tools().iter().map(|tool| tool.name.as_str()).collect();
    assert!(
        names.contains(&"update_layer")
            && names.contains(&"apply_edit")
            && names.contains(&"get_page")
    );
    assert!(
        !names.contains(&"add_file")
            && !names.contains(&"write_pixels")
            && !names.contains(&"edit_text")
    );
    let update = tools()
        .iter()
        .find(|tool| tool.name == "update_layer")
        .unwrap();
    assert!(!update.input_schema.to_string().contains("\"ingredients\""));
    assert!(update.input_schema["properties"].get("op").is_none());
    assert!(update.input_schema["$defs"]["LayerPatch"].is_object());
    assert!(!update.description.is_empty());

    let call = tool_call("set_text", json!({"layer": LayerId::new(), "text": "hi"})).unwrap();
    assert!(matches!(call, ToolCall::Edit { label: None, ref ops } if ops.len() == 1));
    assert!(
        tool_call(
            "set_text",
            json!({"op": "remove_layer", "layer": LayerId::new()})
        )
        .is_err()
    );
    assert!(tool_call("add_file", json!({"media_type": "x", "bytes": []})).is_err());
    let call = tool_call("get_page", json!({"page": PageId::new()})).unwrap();
    assert!(matches!(call, ToolCall::Query(Query::GetPage { .. })));
    let call = tool_call(
        "apply_edit",
        json!({"label": "x", "ops": [{"op": "remove_layer", "layer": "$a"}]}),
    )
    .unwrap();
    assert!(matches!(call, ToolCall::Edit { label: Some(ref label), .. } if label == "x"));
}

#[test]
fn compact_answers_drop_hashes_but_keep_attributes() {
    let (_dir, mut session, page) = session();
    session
        .apply(edit(
            &session,
            user(),
            vec![op(
                json!({"op": "add_layer", "page": page, "index": 0, "layer": {
                    "kind": "group", "attributes": {"ns": {"hash": "keep", "origin": [1]}},
                }}),
            )],
        ))
        .unwrap();
    let answer = compact(&session.query(Query::GetPage { page }).unwrap());
    assert!(answer["layers"][0].get("hash").is_none());
    assert_eq!(
        answer["layers"][0]["layer"]["attributes"]["ns"],
        json!({"hash": "keep", "origin": [1]})
    );
    let project = compact(&session.query(Query::GetProject).unwrap());
    assert!(
        project["pages"][0]["layers"][0].get("hash").is_none(),
        "{project}"
    );
}
