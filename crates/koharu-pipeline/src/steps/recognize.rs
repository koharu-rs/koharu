//! Recognition: each block's source wording, read from the scan inside its
//! source's frame, with the glyphs of the page's other blocks there blanked
//! in the crop's median colour: a frame can reach over a neighbour's
//! lettering, which would be read as this block's. One call reads every page
//! the task holds.

use anyhow::Result;
use image::{DynamicImage, GrayImage, Rgb, RgbImage};
use koharu_scene::{LayerId, Op, Project, Rect};
use serde_json::json;

use super::{Member, Stages, action, blocks, bounds, clamp, is_block, on_page, scan};
use crate::{Recognizer, Step, models::Models};

pub(crate) struct Recognize {
    model: Recognizer,
}

impl Recognize {
    pub(crate) fn new(model: Recognizer) -> Self {
        Self { model }
    }
}

impl Stages for Recognize {
    /// Each block and the scan inside its source's frame.
    type Input = Vec<(LayerId, DynamicImage)>;
    type Output = Vec<(LayerId, String)>;
    type Ready = Self::Output;

    const STEP: Step = Step::Recognize;

    fn read(&self, member: &Member) -> Result<Option<Self::Input>> {
        let Some((project, scan)) = scan(member)? else {
            return Ok(None);
        };
        let (width, height) = (scan.width(), scan.height());
        let masks: Vec<(LayerId, Rect, GrayImage)> = {
            let session = member.session.lock();
            blocks(&project, member.page, None)
                .into_iter()
                .filter_map(|(id, block)| {
                    let tiles = session.tiles(block.mask.id, None).ok()?;
                    let (rect, mask) = on_page(&tiles, width, height)?;
                    Some((id, rect, mask))
                })
                .collect()
        };
        let crops: Self::Input = blocks(&project, member.page, member.blocks.as_deref())
            .into_iter()
            .filter_map(|(id, block)| {
                let rect = clamp(bounds(&block.source)?, width, height)?;
                let mut crop = scan
                    .crop_imm(rect.x as u32, rect.y as u32, rect.width, rect.height)
                    .to_rgb8();
                blank(&mut crop, rect, id, &masks);
                Some((id, DynamicImage::ImageRgb8(crop)))
            })
            .collect();
        Ok((!crops.is_empty()).then_some(crops))
    }

    fn infer(
        &mut self,
        models: &mut Models,
        inputs: Vec<Self::Input>,
    ) -> Result<Vec<Self::Output>> {
        let counts: Vec<usize> = inputs.iter().map(Vec::len).collect();
        let (blocks, images): (Vec<LayerId>, Vec<DynamicImage>) =
            inputs.into_iter().flatten().unzip();
        let recognizer = models.recognizer(self.model)?;
        let _span = tracing::info_span!("ocr", crops = images.len()).entered();
        let texts = recognizer.recognize(&images)?;
        anyhow::ensure!(
            texts.len() == images.len(),
            "{} texts for {} blocks",
            texts.len(),
            images.len()
        );
        let mut read = blocks.into_iter().zip(texts);
        Ok(counts
            .into_iter()
            .map(|count| read.by_ref().take(count).collect())
            .collect())
    }

    fn finish(&mut self, output: Self::Output) -> Result<Self::Ready> {
        Ok(output)
    }

    fn write(&self, project: &Project, output: Self::Ready) -> Result<Vec<Op>> {
        let model = self.model.to_string();
        let language = self.model.language();
        let mut ops = Vec::new();
        for (block, text) in output {
            if !is_block(project, block) {
                continue;
            }
            ops.push(Op::UpdateLayer {
                layer: block.into(),
                patch: json!({"source": {"content": text, "language": language}}),
            });
            ops.push(Op::SetAttribute {
                layer: block.into(),
                namespace: "c2pa".to_owned(),
                key: "ocr".to_owned(),
                value: Some(action("c2pa.created", &model, None)),
            });
        }
        Ok(ops)
    }
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
