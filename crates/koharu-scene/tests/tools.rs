//! The tools generated for agents and MCP clients, and the answers they get.

mod common;
use common::*;
use koharu_scene::{Answer, LayerId, PageId, Query, ToolCall, Uuid, tool_call, tools};
use serde_json::json;

#[test]
fn schemas_stand_alone() {
    for tool in tools() {
        let schema = tool.input_schema.to_string();
        assert!(
            !schema.contains("$ref") && !schema.contains("$defs"),
            "{}",
            tool.name
        );
    }
}

#[test]
fn tools_cover_what_agents_send() {
    let names: Vec<&str> = tools().iter().map(|tool| tool.name.as_str()).collect();
    for name in [
        "update_layer",
        "apply_edit",
        "get_page",
        "set_text",
        "set_attribute",
    ] {
        assert!(names.contains(&name), "{name}");
    }
    for name in ["add_file", "write_pixels", "set_rich_text", "add_page"] {
        assert!(!names.contains(&name), "{name}");
    }
    let update = tools()
        .iter()
        .find(|tool| tool.name == "update_layer")
        .expect("update_layer");
    assert!(update.input_schema["properties"].get("op").is_none());
    let patch = &update.input_schema["properties"]["patch"];
    assert!(patch.to_string().contains("translation"), "{patch}");
    assert!(patch["properties"].get("attributes").is_none(), "{patch}");
    assert!(!update.description.is_empty());

    let call = tool_call(
        "set_text",
        json!({"layer": LayerId(Uuid::now_v7()), "text": "hi"}),
    )
    .expect("a call");
    assert!(matches!(call, ToolCall::Edit { label: None, ref ops } if ops.len() == 1));
    assert!(
        tool_call(
            "set_text",
            json!({"op": "remove_layer", "layer": LayerId(Uuid::now_v7())})
        )
        .is_err()
    );
    assert!(tool_call("add_file", json!({"bytes": []})).is_err());
    let call = tool_call("get_page", json!({"page": PageId(Uuid::now_v7())})).expect("a call");
    assert!(matches!(call, ToolCall::Query(Query::GetPage { .. })));
    let call = tool_call(
        "apply_edit",
        json!({"label": "x", "ops": [{"op": "remove_layer", "layer": "$a"}]}),
    )
    .expect("a call");
    assert!(matches!(call, ToolCall::Edit { label: Some(ref label), .. } if label == "x"));
    assert!(
        tool_call(
            "apply_edit",
            json!({"label": "x", "ops": [{"op": "add_file", "bytes": []}]})
        )
        .is_err()
    );
}

#[test]
fn every_null_the_patch_schema_allows_is_a_patch_that_applies() {
    let update = tools()
        .iter()
        .find(|tool| tool.name == "update_layer")
        .expect("update_layer");
    let patch = &update.input_schema["properties"]["patch"];
    let variants = patch["anyOf"].as_array().expect("the kinds' variants");
    let blocks = variants
        .iter()
        .find(|variant| variant["properties"].get("translation").is_some())
        .expect("the block's variant");
    let nullable = |properties: &serde_json::Value| -> Vec<String> {
        let properties = properties.as_object().expect("properties");
        properties
            .iter()
            .filter(|(_, schema)| {
                schema["anyOf"]
                    .as_array()
                    .is_some_and(|options| options.iter().any(|option| option["type"] == "null"))
            })
            .map(|(key, _)| key.clone())
            .collect()
    };
    let mut keys = nullable(&patch["properties"]);
    keys.extend(nullable(&blocks["properties"]));
    assert!(keys.contains(&"translation".to_owned()), "{keys:?}");

    let (_dir, mut session, page) = session();
    let layer = add(&mut session, page, None, block(0.0));
    for key in keys {
        let result = apply(
            &mut session,
            vec![json!({"op": "update_layer", "layer": layer, "patch": {key.clone(): null}})],
        );
        assert!(result.is_ok(), "{key}: {result:?}");
    }
}

#[test]
fn agents_patch_a_block_and_read_it_in_its_page() {
    let (_dir, mut session, page) = session();
    let panel = add(
        &mut session,
        page,
        None,
        json!({"kind": "panel", "outline": outline()}),
    );
    let balloon = add(
        &mut session,
        page,
        Some(panel),
        json!({"kind": "balloon", "outline": outline()}),
    );
    add(&mut session, page, Some(balloon), block(0.0));
    let second = add(&mut session, page, Some(balloon), block(10.0));

    let ToolCall::Edit { ops, .. } = tool_call(
        "update_layer",
        json!({"layer": second, "patch": {"translation": {"direction": "vertical"}}}),
    )
    .expect("a call") else {
        panic!("not an edit")
    };
    session.apply(edit(ops)).expect("an edit");

    let Answer::Page(answer) = session.query(Query::GetPage { page }).expect("an answer") else {
        panic!("not a page")
    };
    assert_eq!(answer.id, page);
    let value = serde_json::to_value(&answer).expect("an answer serializes");
    let found = |value: &serde_json::Value, id: LayerId| {
        fn search<'v>(layers: &'v serde_json::Value, id: &str) -> Option<&'v serde_json::Value> {
            layers.as_array()?.iter().find_map(|layer| {
                (layer["id"] == id)
                    .then_some(layer)
                    .or_else(|| search(&layer["children"], id))
            })
        }
        search(&value["layers"], &id.to_string()).cloned()
    };
    let second = found(&value, second).expect("the second block, inside its balloon");
    assert_eq!(second["translation"]["direction"], "vertical", "{value}");
    let Answer::Project(project) = session.query(Query::GetProject).expect("an answer") else {
        panic!("not a project")
    };
    assert_eq!(
        (project.version.as_str(), project.pages.len()),
        ("1.0.0", 1)
    );
}
