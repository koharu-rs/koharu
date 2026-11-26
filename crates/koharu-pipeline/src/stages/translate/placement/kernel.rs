//! Which kernel's style a translation is lettered in: its source's, as
//! detection read it and pooled with the project's readings of its kind, when
//! detection is sure of its group, a sound effect's as a text's; a speech
//! balloon's text read as antique or mincho in the project's dialogue face of
//! the two, as a book letters its dialogue in one and the font network can't
//! tell them apart; and bold when its source is heavier than the book's
//! dialogue, as a shout's is.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result};
use icu_properties::{CodePointMapData, props::Script};
use imageproc::{geometry::approximate_polygon_dp, point::Point};
use koharu_ml::yuzumarker_font_detection as fonts;
use koharu_scene::{Block, Content, Kernel, LayerId, Project, Role};
use kurbo::{BezPath, Shape as _};
use serde::Deserialize;

use crate::stages::detect::Detection;

/// The least probability of a source's likeliest group of kernels that
/// letters it in that group's likeliest kernel's style rather than antique's.
/// A group's, since detection splits a face among the kernels of its look.
const SURE: f64 = 0.6;
/// The least probability of the mincho kernels that letters a text as them.
/// Antique kana with gothic kanji, the commonest dialogue face, reads as them,
/// as the network knows 21 antique fonts against 1,189 mincho ones, often
/// past 0.9: of Manga109 texts read at 0.9 and up, every fourth book's
/// middle pages, 91 were in a mincho face and 48 in an antique one, and from
/// 0.995 on, 87 and 27. Neither a text's share of kanji nor a book's share of
/// mincho readings told the two apart.
const MINCHO: f64 = 0.995;
/// How much heavier than the book's dialogue a source's stroke is to be bold.
const HEAVY: f64 = 1.3;
const BOLD: u16 = 700;
/// How far a block's reading leans to its project's reading of its kind: the
/// power the others' mean takes in their product. On Manga109, with the five
/// crops detection averages, 0.5 left a style changed by a crop's margin for
/// none of the balloon texts, 5% of free texts, and 4% of sound effects.
const POOL: f64 = 0.5;
/// The share of Han among a text's Han and kana from which it's kanji-heavy,
/// and the least such balloon texts, and their least mean reading as the
/// mincho kernels, that set a project's dialogue in mincho. Antique sets its
/// kanji in gothic, so its kanji-heavy texts seldom read as mincho, where its
/// kana-heavy ones often do: on Manga109, the books whose dialogue is in
/// mincho read 0.92 and 1.00, those in antique 0.26 at most, and Dorohedoro's
/// first volume, in antique, 0.22 over 112 texts.
const KANJI: f64 = 0.4;
const KANJI_TEXTS: usize = 5;
const MINCHO_DIALOGUE: f64 = 0.7;
/// The least share of a balloon's outline, simplified to within a fiftieth of
/// its diagonal, running within `SQUARE` degrees of horizontal or vertical that
/// makes it a box. Narration and monologue sit in boxes, one or two run
/// together, often in a face of their own, where speech balloons curve and a
/// shout's spikes run diagonal: in chapter 119 of Tensei Goblin, 22 of the 28
/// balloon texts read as mincho, its narration, were in boxes, against 1 of
/// the 36 read as antique and none of the 10 gothic shouts; in Dorohedoro's
/// first volume, 22 of the 474 read as mincho.
const BOX: f64 = 0.8;
const SQUARE: f64 = 10.0;

/// The style a font kernel is lettered in: its own.
fn style(kernel: fonts::Kernel) -> Kernel {
    use fonts::Kernel as Font;
    match kernel {
        Font::Antique => Kernel::Antique,
        Font::Mincho => Kernel::Mincho,
        Font::Fangsong => Kernel::Fangsong,
        Font::Gothic => Kernel::Gothic,
        Font::Rounded => Kernel::Rounded,
        Font::Kaisho => Kernel::Kaisho,
        Font::Gyokai => Kernel::Gyokai,
        Font::Gyosho => Kernel::Gyosho,
        Font::Sosho => Kernel::Sosho,
        Font::Reisho => Kernel::Reisho,
        Font::Tensho => Kernel::Tensho,
        Font::Kointai => Kernel::Kointai,
        Font::Weibei => Kernel::Weibei,
        Font::Textbook => Kernel::Textbook,
        Font::Brush => Kernel::Brush,
        Font::Handwriting => Kernel::Handwriting,
        Font::Pop => Kernel::Pop,
        Font::Design => Kernel::Design,
        Font::Decorative => Kernel::Decorative,
    }
}

/// What a block is, which a project letters alike: a text in a speech
/// balloon, a text in a box, a text outside both, or a sound effect.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Balloon,
    Box,
    Free,
    Sfx,
}

/// A block's reading: how likely each style is to be its source's, and its
/// stroke over its size, what it is, and whether its source is kanji-heavy.
struct Read {
    kernels: BTreeMap<Kernel, f64>,
    stroke: Option<f64>,
    kind: Kind,
    kanji: bool,
}

impl Read {
    /// Its likeliest group, with the probability its kernels sum to, and the
    /// likeliest kernel in it.
    fn top(&self) -> Option<(Kernel, f64, Kernel)> {
        let mut groups = BTreeMap::<Kernel, f64>::new();
        for (&kernel, &probability) in &self.kernels {
            *groups.entry(kernel.group()).or_default() += probability;
        }
        let (group, sum) = groups.into_iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
        let kernel = (self.kernels.iter())
            .filter(|(kernel, _)| kernel.group() == group)
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(&kernel, _)| kernel)?;
        Some((group, sum, kernel))
    }
}

/// Each block's reading, the project's dialogue face, and how heavy its
/// dialogue is, read once for a project. The dialogue is its texts in speech
/// balloons, or every text outside them when none is in one: per page, a page
/// of mostly monologue would weigh rounded gothic as its dialogue.
pub(super) struct Kernels {
    reads: BTreeMap<LayerId, Read>,
    face: Kernel,
    /// The median stroke of the dialogue texts read as antique or mincho.
    stroke: Option<f64>,
}

impl Kernels {
    pub(super) fn new(project: &Project) -> Result<Self> {
        let mut reads = BTreeMap::new();
        for page in project.pages() {
            for (id, block) in project.blocks(page.id)? {
                reads.insert(id, read(project, id, block)?);
            }
        }
        let in_balloons = reads.values().any(|read| read.kind == Kind::Balloon);
        let dialogue = |read: &&Read| match read.kind {
            Kind::Balloon => true,
            Kind::Box | Kind::Free => !in_balloons,
            Kind::Sfx => false,
        };
        let face = face(&reads.values().filter(dialogue).collect::<Vec<_>>());
        pool(&mut reads);
        let strokes = (reads.values().filter(dialogue))
            .filter(|read| read.top().is_some_and(|(group, ..)| confused(group)))
            .filter_map(|read| read.stroke)
            .collect();
        Ok(Self {
            reads,
            face,
            stroke: super::size::median(strokes),
        })
    }

    /// The style block `id` of the project is lettered in, and the weight it's
    /// set at when its source is heavier than the book's dialogue. A speech
    /// balloon's text read as antique or mincho takes the project's dialogue
    /// face, as a book's dialogue keeps one face from balloon to balloon where
    /// the network reads antique's kana as mincho; any other block keeps its
    /// own, antique's when detection isn't sure of it, so a gothic shout, a
    /// rounded thought, or a box's narration keeps its face.
    pub(super) fn of(&self, id: LayerId) -> (Kernel, Option<u16>) {
        let Some(read) = self.reads.get(&id) else {
            return (Kernel::Antique, None);
        };
        let kernel = match read.top() {
            Some((Kernel::Mincho, sum, _)) if sum < MINCHO => Kernel::Antique,
            Some((_, sum, kernel)) if sum >= SURE => kernel,
            _ => Kernel::Antique,
        };
        let kernel = match read.kind {
            Kind::Balloon if confused(kernel.group()) => self.face,
            _ => kernel,
        };
        let heavy =
            (read.stroke.zip(self.stroke)).is_some_and(|(own, dialogue)| own >= HEAVY * dialogue);
        (kernel, heavy.then_some(BOLD))
    }
}

/// Whether `group` is antique or mincho, which the font network confuses.
fn confused(group: Kernel) -> bool {
    matches!(group, Kernel::Antique | Kernel::Mincho)
}

/// The face of a project's `dialogue`: mincho when its kanji-heavy texts read
/// as the mincho kernels, else antique.
fn face(dialogue: &[&Read]) -> Kernel {
    let group = |read: &Read, groups: &[Kernel]| -> f64 {
        (read.kernels.iter())
            .filter(|(kernel, _)| groups.contains(&kernel.group()))
            .map(|(_, probability)| probability)
            .sum()
    };
    let kanji: Vec<f64> = (dialogue.iter())
        .filter(|read| read.kanji)
        .map(|read| group(read, &[Kernel::Mincho]))
        .collect();
    if kanji.len() >= KANJI_TEXTS
        && kanji.iter().sum::<f64>() / kanji.len() as f64 >= MINCHO_DIALOGUE
    {
        Kernel::Mincho
    } else {
        Kernel::Antique
    }
}

/// Whether a balloon's `outline` is a box: `BOX` or more of it, simplified,
/// runs within `SQUARE` degrees of horizontal or vertical.
fn boxed(outline: &BezPath) -> bool {
    let points: Vec<Point<f64>> = (outline.elements().iter())
        .filter_map(|element| element.end_point())
        .map(|point| Point::new(point.x, point.y))
        .collect();
    if points.len() < 4 {
        return false;
    }
    let bounds = outline.bounding_box();
    let epsilon = bounds.width().hypot(bounds.height()) / 50.0;
    // Simplified as two halves split at the point farthest from the first:
    // imageproc's closed simplification measures from a chord between the
    // first point and the last, which a closed outline makes a point.
    let start = points[0];
    let far = (0..points.len())
        .max_by(|&a, &b| {
            let distance = |at: usize| (points[at].x - start.x).hypot(points[at].y - start.y);
            distance(a).total_cmp(&distance(b))
        })
        .unwrap_or(0);
    if far == 0 {
        return false;
    }
    let back: Vec<Point<f64>> = points[far..].iter().copied().chain([start]).collect();
    let mut ring = approximate_polygon_dp(&points[..=far], epsilon, false);
    ring.pop();
    ring.extend(approximate_polygon_dp(&back, epsilon, false));
    ring.pop();
    let (mut total, mut square) = (0.0, 0.0);
    for (from, to) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        let (across, down) = ((to.x - from.x).abs(), (to.y - from.y).abs());
        let length = across.hypot(down);
        let angle = down.atan2(across).to_degrees();
        total += length;
        if angle <= SQUARE || angle >= 90.0 - SQUARE {
            square += length;
        }
    }
    total > 0.0 && square >= BOX * total
}

/// Each reading times the mean of its project's other readings of its kind,
/// to the power `POOL`, so a reading the crop swings doesn't stand alone.
fn pool(reads: &mut BTreeMap<LayerId, Read>) {
    let mut sums = BTreeMap::<Kind, (BTreeMap<Kernel, f64>, f64)>::new();
    for read in reads.values().filter(|read| !read.kernels.is_empty()) {
        let (sum, count) = sums.entry(read.kind).or_default();
        for (&kernel, &probability) in &read.kernels {
            *sum.entry(kernel).or_default() += probability;
        }
        *count += 1.0;
    }
    for read in reads.values_mut() {
        let Some((sum, count)) = sums.get(&read.kind).filter(|(_, count)| *count >= 2.0) else {
            continue;
        };
        for (kernel, probability) in &mut read.kernels {
            let others = (sum[kernel] - *probability) / (count - 1.0);
            *probability *= (others + 1e-3).powf(POOL);
        }
        let total: f64 = read.kernels.values().sum();
        if total > 0.0 {
            read.kernels
                .values_mut()
                .for_each(|probability| *probability /= total);
        }
    }
}

/// Block `id`'s reading, which fails on a detection it can't read, with the
/// mincho kernels, which kana alone can't tell from antique, read as antique
/// when its source has no Han character.
fn read(project: &Project, id: LayerId, block: &Block) -> Result<Read> {
    let Detection {
        kernels: found,
        stroke,
        ..
    } = match project
        .layer(id)?
        .attributes
        .get("koharu")
        .and_then(|values| values.get("detection"))
    {
        Some(value) => Detection::deserialize(value)
            .with_context(|| format!("block {id}'s detection can't be read"))?,
        None => Detection::default(),
    };
    let scripts = CodePointMapData::<Script>::new();
    let (mut han, mut kana) = (0usize, 0usize);
    for c in block.source.content.text().chars() {
        match scripts.get(c) {
            Script::Han => han += 1,
            Script::Hiragana | Script::Katakana => kana += 1,
            _ => {}
        }
    }
    let mut kernels = BTreeMap::<Kernel, f64>::new();
    for (kernel, probability) in found {
        let kernel = match style(kernel) {
            kernel if kernel.group() == Kernel::Mincho && han == 0 => Kernel::Antique,
            kernel => kernel,
        };
        *kernels.entry(kernel).or_default() += probability;
    }
    let (_, holder) = project.parent(id)?;
    let balloon = holder.and_then(|holder| match &holder.content {
        Content::Balloon(balloon) => Some(balloon),
        _ => None,
    });
    Ok(Read {
        kernels,
        stroke,
        kind: match (block.role, balloon) {
            (Role::Sfx, _) => Kind::Sfx,
            (Role::Text, Some(balloon)) if boxed(&BezPath::from(&balloon.outline)) => Kind::Box,
            (Role::Text, Some(_)) => Kind::Balloon,
            (Role::Text, None) => Kind::Free,
        },
        kanji: han > 0 && han as f64 >= KANJI * (han + kana) as f64,
    })
}
