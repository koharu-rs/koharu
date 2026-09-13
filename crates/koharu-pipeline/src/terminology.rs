use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use koharu_scene::{
    EntityId, GlossaryCandidate, GlossaryCategory, Snapshot, normalize_term, normalize_text,
};

use crate::StopToken;

#[derive(Default)]
struct Statistics {
    count: u32,
    pages: BTreeSet<EntityId>,
    category: GlossaryCategory,
    distinctive: bool,
}

pub fn analyze_terminology(
    snapshot: &Snapshot,
    pages: &[EntityId],
    stop: &StopToken,
    mut progress: impl FnMut(usize),
) -> Result<Option<Vec<GlossaryCandidate>>> {
    let mut terms = BTreeMap::<String, Statistics>::new();
    for (index, page) in pages.iter().enumerate() {
        if stop.stopped() {
            return Ok(None);
        }
        if let Some(group) = snapshot.page(*page)?.text_group()? {
            let mut seen = BTreeSet::new();
            for layer in group.text_layers()? {
                if stop.stopped() {
                    return Ok(None);
                }
                let content = layer.content()?;
                if !seen.insert(content.id()) {
                    continue;
                }
                if let Some(source) = content.source()? {
                    collect(&source.text.value, *page, &mut terms)?;
                }
            }
        }
        progress(index + 1);
    }
    Ok(Some(candidates(terms)))
}

fn collect(text: &str, page: EntityId, terms: &mut BTreeMap<String, Statistics>) -> Result<()> {
    let text = normalize_text(text);
    let mut start = 0;
    let mut kind = 0;
    for (offset, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        let next = script(character);
        if next != kind || next == 0 {
            if kind != 0 {
                record(&text[start..offset], &text[offset..], page, terms)?;
            }
            start = offset;
            kind = next;
        }
    }
    Ok(())
}

fn script(character: char) -> u8 {
    match character {
        '\u{3400}'..='\u{9fff}' | '々' => 1,
        '\u{30a1}'..='\u{30fa}' | 'ー' | '・' => 2,
        'a'..='z' | 'A'..='Z' | '-' => 3,
        _ => 0,
    }
}

fn record(
    source: &str,
    following: &str,
    page: EntityId,
    terms: &mut BTreeMap<String, Statistics>,
) -> Result<()> {
    let source = normalize_term(source).trim_matches(['・', '-']).to_owned();
    if !(2..=32).contains(&source.chars().count()) {
        return Ok(());
    }
    if [
        "今日",
        "明日",
        "自分",
        "何処",
        "本当",
        "全部",
        "大丈夫",
        "This",
        "That",
        "The",
    ]
    .contains(&source.as_str())
    {
        return Ok(());
    }
    let (category, distinctive) = classify(&source, following);
    if script(source.chars().next().unwrap()) == 3 && !distinctive {
        return Ok(());
    }
    if !terms.contains_key(&source) && terms.len() >= 100_000 {
        bail!("terminology analysis found over 100000 distinct terms; use a smaller page scope");
    }
    let term = terms.entry(source).or_default();
    term.count = term.count.saturating_add(1);
    term.pages.insert(page);
    if distinctive || !term.distinctive {
        term.category = category;
    }
    term.distinctive |= distinctive;
    Ok(())
}

fn classify(source: &str, following: &str) -> (GlossaryCategory, bool) {
    use GlossaryCategory::*;
    if ["さん", "ちゃん", "くん", "君", "様", "氏", "殿"]
        .iter()
        .any(|suffix| following.starts_with(*suffix))
        || [
            "高橋", "田中", "佐藤", "鈴木", "渡辺", "伊藤", "加藤", "山田", "山本", "小林", "中村",
            "吉田",
        ]
        .contains(&source)
    {
        return (Person, true);
    }
    for (suffixes, category) in [
        (
            &["院", "学園", "会社", "組合", "協会", "騎士団", "軍"] as &[_],
            Organization,
        ),
        (&["隊長", "先生", "陛下", "博士", "団長", "姫", "王"], Title),
        (&["市", "町", "村", "国", "城", "島"], Place),
        (&["術", "法", "斬", "撃"], Skill),
        (&["剣", "杖", "薬", "鎧"], Item),
    ] {
        if suffixes.iter().any(|suffix| source.ends_with(*suffix)) {
            return (category, true);
        }
    }
    let katakana = source.chars().all(|character| script(character) == 2);
    let capitalized = source
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_uppercase());
    (Terminology, katakana || capitalized)
}

fn candidates(terms: BTreeMap<String, Statistics>) -> Vec<GlossaryCandidate> {
    let mut terms = terms
        .into_iter()
        .filter(|(_, value)| value.distinctive || value.count >= 2 || value.pages.len() >= 2)
        .collect::<Vec<_>>();
    terms.sort_by(|a, b| {
        b.1.pages
            .len()
            .cmp(&a.1.pages.len())
            .then_with(|| b.1.distinctive.cmp(&a.1.distinctive))
            .then_with(|| b.1.count.cmp(&a.1.count))
            .then_with(|| a.0.cmp(&b.0))
    });
    terms
        .into_iter()
        .take(5000)
        .map(|(source, value)| GlossaryCandidate {
            source,
            suggested_target: String::new(),
            category: value.category,
            occurrences: value.count,
            page_count: value.pages.len() as u32,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cross_page_statistics_normalization_and_noun_cues() {
        let mut terms = BTreeMap::new();
        let page = EntityId::new();
        collect("高橋は高橋です。魔導院のｴｰﾃﾙ。", page, &mut terms).unwrap();
        collect(
            "「高橋」さん、エーテルを魔導院へ。",
            EntityId::new(),
            &mut terms,
        )
        .unwrap();
        let candidates = candidates(terms);
        let name = candidates
            .iter()
            .find(|entry| entry.source == "高橋")
            .unwrap();
        assert_eq!(
            (name.occurrences, name.page_count, name.category),
            (3, 2, GlossaryCategory::Person)
        );
        let ether = candidates
            .iter()
            .find(|entry| entry.source == "エーテル")
            .unwrap();
        assert_eq!((ether.occurrences, ether.page_count), (2, 2));
        assert_eq!(
            candidates
                .iter()
                .find(|entry| entry.source == "魔導院")
                .unwrap()
                .category,
            GlossaryCategory::Organization
        );
    }

    #[tokio::test]
    async fn analysis_cancels_without_modifying_scene() {
        let session = koharu_scene::Session::memory().await.unwrap();
        let stop = StopToken::default();
        stop.stop();
        let result =
            analyze_terminology(&session.snapshot(), &[EntityId::new()], &stop, |_| {}).unwrap();
        assert!(result.is_none());
        assert!(
            session
                .snapshot()
                .project_component::<koharu_scene::ProjectGlossary>()
                .unwrap()
                .is_none()
        );
    }
}
