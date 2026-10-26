//! What an agent or MCP client does: list the tools, call them, and read
//! answers without hashes. The host runs each call on the session as the
//! user, names the agent with `via`, and gives single-op tools its own
//! translated undo label.
//!
//! `cargo run -p koharu-scene --example tools`

use koharu_scene::{
    Author, Edit, Error, PageId, Session, Size, SoftwareAgent, ToolCall, compact, tool_call, tools,
};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Runs one tool call the way the app's host does, and returns what the agent
/// sees.
fn run(session: &mut Session, name: &str, arguments: Value) -> koharu_scene::Result<Value> {
    match tool_call(name, arguments)? {
        ToolCall::Edit { label, ops } => {
            let change = session.apply(Edit {
                project: session.id(),
                // `apply_edit` carries the agent's label; for a single op the
                // app supplies a translated one.
                label: label.unwrap_or_else(|| "Agent edit".to_owned()),
                author: Author::User {
                    via: Some(SoftwareAgent {
                        name: "assistant".to_owned(),
                        version: None,
                    }),
                },
                reads: Vec::new(),
                ops,
            })?;
            Ok(json!({"created": change.created}))
        }
        ToolCall::Query(query) => Ok(compact(&session.query(query)?)),
    }
}

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("chapter.koharu"))?;
    // Pages come from imports, not from tools.
    let change = session.apply(Edit {
        project: session.id(),
        label: "Import".to_owned(),
        author: Author::User { via: None },
        reads: Vec::new(),
        ops: vec![koharu_scene::Op::AddPage {
            id: None,
            index: 0,
            size: Size {
                width: 1200,
                height: 1700,
            },
        }],
    })?;
    let page: PageId = change.pages.added[0];

    // One tool per op an agent may send, `apply_edit` for several at once,
    // and one per query. Ops that carry bytes, editor deltas, or pixels are
    // not tools.
    for tool in tools() {
        let kind = if tool.writes { "edit" } else { "read" };
        let summary = tool.description.lines().next().unwrap_or_default();
        println!("{:<16} {kind}  {summary}", tool.name);
    }
    let names: Vec<&str> = tools().iter().map(|tool| tool.name.as_str()).collect();
    assert!(
        names.contains(&"add_layer")
            && names.contains(&"apply_edit")
            && names.contains(&"get_page")
    );
    assert!(!names.contains(&"add_page") && !names.contains(&"write_pixels"));

    // A single op.
    let created = run(
        &mut session,
        "add_layer",
        json!({
            "page": page, "index": 0, "layer": {"kind": "text", "content": "Hi!"},
        }),
    )?;
    let text = created["created"][0]["id"].clone();

    // Several ops as one undo step; later ops name earlier ones by `$label`.
    run(
        &mut session,
        "apply_edit",
        json!({
            "label": "Add a balloon",
            "ops": [
                {"op": "add_layer", "id": "$balloon", "page": page, "index": 1, "layer": {"kind": "group", "name": "Balloon"}},
                {"op": "add_layer", "page": page, "parent": "$balloon", "index": 0, "layer": {
                    "kind": "shape", "path": "M0 0 H100 V60 H0 Z", "fill": [1.0, 1.0, 1.0, 1.0],
                }},
                {"op": "move_layer", "layer": text, "parent": "$balloon", "index": 1},
            ],
        }),
    )?;
    assert_eq!(session.history().undo.as_deref(), Some("Add a balloon"));
    run(
        &mut session,
        "update_layer",
        json!({"layer": text, "patch": {"style": {"chars": {"size": 26.0}}}}),
    )?;

    // Answers come compacted: no hashes, and each action reduced to what it
    // was and who did it.
    let answer = run(&mut session, "get_page", json!({"page": page}))?;
    assert!(answer["layers"][0].get("hash").is_none());
    println!("{}", serde_json::to_string_pretty(&answer)?);

    // Mistakes come back as errors the agent can read and correct.
    let error = run(&mut session, "rename_layer", json!({})).unwrap_err();
    assert!(matches!(error, Error::Invalid { .. }));
    let error = run(
        &mut session,
        "update_layer",
        json!({"layer": text, "patch": {"colour": "red"}}),
    )
    .unwrap_err();
    println!("{}", serde_json::to_value(&error)?);
    Ok(())
}
