//! What an agent or MCP client does: list the tools, call them, and read
//! their answers, which are the project's own values.
//!
//! `cargo run -p koharu-scene --example tools`

use koharu_scene::{Answer, Edit, Session, ToolCall, tool_call, tools};
use serde_json::json;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Runs a call as the host does: an edit gets a label, and a query an answer.
fn call(
    session: &mut Session,
    name: &str,
    arguments: serde_json::Value,
) -> Result<serde_json::Value> {
    Ok(match tool_call(name, arguments)? {
        ToolCall::Edit { label, ops } => {
            let change = session.apply(Edit {
                label: label.unwrap_or_else(|| name.to_owned()),
                ops,
            })?;
            serde_json::to_value(change)?
        }
        ToolCall::Query(query) => serde_json::to_value(session.query(query)?)?,
    })
}

fn main() -> Result<()> {
    for tool in tools() {
        println!(
            "{} ({}): {}",
            tool.name,
            if tool.writes { "writes" } else { "reads" },
            tool.description
        );
    }

    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("agent.khr"))?;
    let change = session.apply(Edit {
        label: "Import".into(),
        ops: vec![serde_json::from_value(
            json!({"op": "add_page", "index": 0, "size": {"width": 400, "height": 600}}),
        )?],
    })?;
    let page = change.pages.added[0];

    let added = call(
        &mut session,
        "apply_edit",
        json!({"label": "Add texts", "ops": [
            {"op": "add_layer", "id": "$b", "page": page, "index": 0, "layer": {"kind": "balloon", "outline": "M0 0 L200 0 L200 200 L0 200 Z"}},
            {"op": "add_layer", "id": "$t", "page": page, "parent": "$b", "index": 0, "layer": {"kind": "block", "source": {"frame": {"box": {"bounds": {"x": 10.0, "y": 10.0, "width": 40.0, "height": 80.0}}}, "content": "ありがとう"}}},
        ]}),
    )?;
    let block = added["created"]
        .as_array()
        .and_then(|created| created.iter().find(|created| created["label"] == "t"))
        .map(|created| created["id"].clone())
        .ok_or("no block")?;
    call(
        &mut session,
        "set_text",
        json!({"layer": block, "text": "Thank you"}),
    )?;
    call(
        &mut session,
        "update_layer",
        json!({"layer": block, "patch": {"translation": {"language": "en"}}}),
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&call(&mut session, "get_layer", json!({"layer": block}))?)?
    );

    let Answer::History(history) = session.query(koharu_scene::Query::GetHistory)? else {
        return Err("not a history".into());
    };
    println!("undo: {:?}", history.undo());
    Ok(())
}
