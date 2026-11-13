//! Recognition: each block's source wording, read from the scan inside its
//! source's frame, with the glyphs of the page's other blocks there blanked
//! in the crop's median colour: a frame can reach over a neighbour's
//! lettering, which would be read as this block's. One call reads every page
//! the task holds.

mod model;

use image::{DynamicImage, GrayImage, Rgb, RgbImage};
use koharu_scene::{Content, LayerId, Rect};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub use self::model::Recognizer;
use super::{Member, Task, action, blocking, key};
use crate::{PipelineConfig, Stage};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(specta::Type)]
pub struct RecognizerConfig {
    pub model: Recognizer,
}

pub(crate) async fn recognize(task: &Task, config: &PipelineConfig) -> anyhow::Result<()> {
    let members = task.members.clone();
    let read = blocking(move || {
        let mut read = Vec::new();
        for member in members {
            let _span =
                tracing::info_span!("read", stage = %Stage::Recognize, page = member.index + 1)
                    .entered();
            if let Some(crops) = crops(&member)? {
                read.push((member, crops));
            }
        }
        Ok(read)
    })
    .await?;
    if read.is_empty() {
        return Ok(());
    }
    let (members, crops): (Vec<Member>, Vec<Vec<(LayerId, DynamicImage)>>) =
        read.into_iter().unzip();
    let key = key(Stage::Recognize, config).expect("a recognizer is Koharu's");
    let model = config.recognizer.model;
    let (texts, language) = task
        .inference(&members.iter().collect::<Vec<_>>(), move |models| {
            let recognizer = models.get(&key, |device| model::load(model, device))?;
            let counts: Vec<usize> = crops.iter().map(Vec::len).collect();
            let (blocks, images): (Vec<LayerId>, Vec<DynamicImage>) =
                crops.into_iter().flatten().unzip();
            let _span = tracing::info_span!("ocr", crops = images.len()).entered();
            let texts = recognizer.inference(&images)?;
            anyhow::ensure!(
                texts.len() == images.len(),
                "{} texts for {} blocks",
                texts.len(),
                images.len()
            );
            let mut read = blocks.into_iter().zip(texts);
            let texts: Vec<Vec<(LayerId, String)>> = counts
                .into_iter()
                .map(|count| read.by_ref().take(count).collect())
                .collect();
            Ok((texts, recognizer.language()))
        })
        .await?;
    let name = model.to_string();
    for (member, texts) in members.iter().zip(texts) {
        let name = name.clone();
        member
            .apply(Stage::Recognize, move |project| {
                let mut ops = Vec::new();
                for (block, text) in texts {
                    if !matches!(
                        project.layer(block).map(|layer| &layer.content),
                        Ok(Content::Block(_))
                    ) {
                        continue;
                    }
                    ops.push(koharu_scene::Op::UpdateLayer {
                        layer: block.into(),
                        patch: json!({"source": {"content": text, "language": language}}),
                    });
                    ops.push(koharu_scene::Op::SetAttribute {
                        layer: block.into(),
                        namespace: "c2pa".to_owned(),
                        key: "ocr".to_owned(),
                        value: Some(action("c2pa.created", &name, None)),
                    });
                }
                Ok(ops)
            })
            .await?;
    }
    Ok(())
}

/// Each of the page's blocks the task works on, and the scan inside its
/// source's frame; `None` when there's none.
fn crops(member: &Member) -> anyhow::Result<Option<Vec<(LayerId, DynamicImage)>>> {
    let Some((project, scan)) = member.scan()? else {
        return Ok(None);
    };
    let (width, height) = (scan.width(), scan.height());
    let masks: Vec<(LayerId, Rect, GrayImage)> = {
        let session = member.session.lock();
        project
            .blocks(member.page)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(id, block)| {
                let tiles = session.tiles(block.mask.id, None).ok()?;
                let (rect, mask) = tiles.coverage(width, height)?;
                Some((id, rect, mask))
            })
            .collect()
    };
    let crops: Vec<(LayerId, DynamicImage)> = member
        .blocks(&project)
        .into_iter()
        .filter_map(|(id, block)| {
            let rect = Rect::covering(block.source.bounds()?, width, height)?;
            let mut crop = scan
                .crop_imm(rect.x as u32, rect.y as u32, rect.width, rect.height)
                .to_rgb8();
            blank(&mut crop, rect, id, &masks);
            Some((id, DynamicImage::ImageRgb8(crop)))
        })
        .collect();
    Ok((!crops.is_empty()).then_some(crops))
}

/// Blanks in `crop`, the scan over `rect`, the pixels of the other blocks'
/// `masks` that block `own`'s mask doesn't hold, in the crop's median colour.
fn blank(crop: &mut RgbImage, rect: Rect, own: LayerId, masks: &[(LayerId, Rect, GrayImage)]) {
    let held = |masks: &[&(LayerId, Rect, GrayImage)], x: i32, y: i32| {
        masks.iter().any(|(_, at, mask)| {
            let (mx, my) = (x - at.x, y - at.y);
            mx >= 0
                && my >= 0
                && (mx as u32) < mask.width()
                && (my as u32) < mask.height()
                && mask.get_pixel(mx as u32, my as u32).0[0] != 0
        })
    };
    let median: [u8; 3] = std::array::from_fn(|channel| {
        let mut values: Vec<u8> = crop.pixels().map(|pixel| pixel.0[channel]).collect();
        values.sort_unstable();
        values.get(values.len() / 2).copied().unwrap_or(u8::MAX)
    });
    let (others, mine): (Vec<_>, Vec<_>) = masks.iter().partition(|(id, ..)| *id != own);
    for (x, y, pixel) in crop.enumerate_pixels_mut() {
        let (px, py) = (rect.x + x as i32, rect.y + y as i32);
        if held(&others, px, py) && !held(&mine, px, py) {
            *pixel = Rgb(median);
        }
    }
}
