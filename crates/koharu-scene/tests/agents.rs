//! What agents send and read: the patch `update_layer` takes, and a page as
//! `get_page` answers it.

mod common;
use common::*;
use koharu_scene::{Answer, LayerId, Query};
use serde_json::json;

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

    apply(
        &mut session,
        vec![json!({
            "op": "update_layer",
            "layer": second,
            "patch": {"translation": {"direction": "vertical"}},
        })],
    )
    .expect("an edit");

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
        ("1.3.0", 1)
    );
}
