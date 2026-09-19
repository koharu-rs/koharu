use std::{collections::BTreeSet, path::PathBuf};

use anyhow::{Context, Result, bail, ensure};
use koharu_ml::glossary_ner::{GlossaryEntityKind, GlossaryNer};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Fixture {
    text: String,
    entities: Vec<ExpectedEntity>,
}

#[derive(Debug, Deserialize)]
struct ExpectedEntity {
    surface: String,
    kind: String,
}

fn kind(name: &str) -> Result<GlossaryEntityKind> {
    Ok(match name {
        "person" => GlossaryEntityKind::Person,
        "place" => GlossaryEntityKind::Place,
        "organization" => GlossaryEntityKind::Organization,
        "item" => GlossaryEntityKind::Item,
        "ability" => GlossaryEntityKind::Ability,
        "work_specific_term" => GlossaryEntityKind::WorkSpecificTerm,
        other => bail!("unknown glossary entity kind {other}"),
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let fixture_path = arguments.next().map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/glossary_ner_ja.json")
    });
    let threshold = arguments
        .next()
        .map(|value| value.to_string_lossy().parse::<f32>())
        .transpose()?
        .unwrap_or(0.3);
    ensure!(
        arguments.next().is_none(),
        "usage: glossary_ner_eval [fixture.json] [threshold]"
    );
    if let Some(store) = std::env::var_os("KOHARU_EVAL_STORE") {
        koharu_runtime::Store::configure(PathBuf::from(store))?;
    }

    let fixtures: Vec<Fixture> = serde_json::from_slice(
        &std::fs::read(&fixture_path)
            .with_context(|| format!("failed to read {}", fixture_path.display()))?,
    )?;
    let runtime = koharu_runtime::Runtime::discover([koharu_runtime::Feature::Torch])?;
    runtime.initialize().await?;
    let model = GlossaryNer::load(koharu_ml::Device::cpu()).await?;

    let mut true_positive = 0usize;
    let mut false_positive = 0usize;
    let mut false_negative = 0usize;
    let mut person_true_positive = 0usize;
    let mut person_false_negative = 0usize;
    for fixture in fixtures {
        let mut gold = BTreeSet::new();
        for expected in fixture.entities {
            let matches = fixture
                .text
                .match_indices(&expected.surface)
                .collect::<Vec<_>>();
            ensure!(
                matches.len() == 1,
                "fixture surface {:?} must occur exactly once in {:?}",
                expected.surface,
                fixture.text
            );
            let start = matches[0].0;
            let entity_kind = kind(&expected.kind)?;
            gold.insert((start, start + expected.surface.len(), entity_kind));
        }
        let predicted = model
            .extract(&fixture.text, threshold)?
            .into_iter()
            .map(|entity| (entity.start, entity.end, entity.kind))
            .collect::<BTreeSet<_>>();
        let missed = gold.difference(&predicted).copied().collect::<Vec<_>>();
        let unexpected = predicted.difference(&gold).copied().collect::<Vec<_>>();
        if !missed.is_empty() || !unexpected.is_empty() {
            eprintln!(
                "text={:?} missed={missed:?} unexpected={unexpected:?}",
                fixture.text
            );
        }
        true_positive += predicted.intersection(&gold).count();
        false_positive += unexpected.len();
        false_negative += missed.len();
        for entity in gold
            .iter()
            .filter(|(_, _, kind)| *kind == GlossaryEntityKind::Person)
        {
            if predicted.contains(entity) {
                person_true_positive += 1;
            } else {
                person_false_negative += 1;
            }
        }
    }

    let precision = ratio(true_positive, true_positive + false_positive);
    let recall = ratio(true_positive, true_positive + false_negative);
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };
    let person_recall = ratio(
        person_true_positive,
        person_true_positive + person_false_negative,
    );
    println!(
        "micro precision={precision:.4} recall={recall:.4} f1={f1:.4}; person recall={person_recall:.4}"
    );
    ensure!(f1 >= 0.70, "micro F1 {f1:.4} is below 0.70");
    ensure!(
        person_recall >= 0.80,
        "person recall {person_recall:.4} is below 0.80"
    );
    Ok(())
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}
