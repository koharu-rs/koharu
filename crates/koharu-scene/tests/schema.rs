//! How new layers parse: what fails, and what each kind starts with. How a
//! patch merges is in `edits.rs`.

use koharu_scene::{Blend, Block, Content, Frame, NewLayer, Role, Text};
use serde_json::{Value, json};

fn source() -> Value {
    json!({"frame": {"box": {"bounds": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 20.0}}}})
}

fn new_layer(value: Value) -> Result<NewLayer, serde_json::Error> {
    serde_json::from_value(value)
}

#[test]
fn a_misspelt_field_fails_to_parse() {
    // Serde doesn't promise that a flattened enum whose variants deny unknown
    // fields refuses a typo beside it, so this pins it.
    assert!(new_layer(json!({"kind": "raster", "opacty": 0.5})).is_err());
    assert!(
        new_layer(json!({"kind": "block", "source": {"frame": source()["frame"], "contnt": ""}}))
            .is_err()
    );
    assert!(new_layer(json!({"kind": "panel", "outline": "M0 0 L1 1 Z", "outlin": ""})).is_err());
    assert!(new_layer(json!({"kind": "baloon", "outline": "M0 0 L1 1 Z"})).is_err());
    assert!(
        new_layer(json!({"kind": "text", "frame": {"box": {"bounds": source()["frame"]["box"]["bounds"], "fitt": "none"}}}))
            .is_err()
    );
}

#[test]
fn opacity_runs_from_zero_to_one() {
    assert!(new_layer(json!({"kind": "group", "opacity": 1.5})).is_err());
    assert!(new_layer(json!({"kind": "group", "opacity": -0.1})).is_err());
    assert!(new_layer(json!({"kind": "group", "opacity": 0.0})).is_ok());
}

#[test]
fn a_block_starts_as_text_with_no_translation() {
    let layer = new_layer(json!({"kind": "block", "source": source()})).expect("a block");
    assert_eq!(
        layer.blend, None,
        "the kind picks pass-through when it's added"
    );
    let Content::Block(block) = layer.content else {
        panic!("{:?}", layer.content)
    };
    let Block {
        role,
        source: Text { frame, content, .. },
        translation,
        mask,
        cleanup,
    } = *block;
    assert_eq!(role, Role::Text);
    assert!(matches!(frame, Frame::Box { .. }));
    assert_eq!(content.text(), "");
    assert_eq!(translation, None);
    assert!(mask.visible && cleanup.visible, "both parts start shown");
    // A misspelt field inside a part is refused, as beside a layer's kind.
    assert!(
        new_layer(json!({"kind": "block", "source": source(), "cleanup": {
            "id": mask.id, "name": "", "visible": true, "locked": false, "opacity": 1.0,
            "blend": "normal", "attributes": {}, "offst": {"x": 0, "y": 0},
        }}))
        .is_err()
    );
    assert!(
        new_layer(json!({"kind": "block"})).is_err(),
        "a block needs a source"
    );
    assert_ne!(Blend::Normal, Blend::PassThrough);
}
