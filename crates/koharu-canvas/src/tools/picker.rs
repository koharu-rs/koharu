//! The colour picker: the page's own pixel under the pointer, as
//! Photoshop's eyedropper samples the image rather than the screen.

use koharu_scene::Rgba;
use kurbo::Rect;

use crate::{Effect, gesture::Context, view::Sample};

/// The page's colour under `sample`, as drawn without the canvas's
/// overlays, over the page's white; nothing off the page.
pub(crate) fn pick(cx: &Context, sample: Sample) -> anyhow::Result<Vec<Effect>> {
    let (page, size) = (sample.page, cx.snapshot.size());
    if !(0.0..size.width).contains(&page.x) || !(0.0..size.height).contains(&page.y) {
        return Ok(Vec::new());
    }
    let pixel = Rect::from_origin_size((page.x.floor(), page.y.floor()), (1.0, 1.0));
    let image = cx.snapshot.frame().rasterize_rect(pixel)?;
    let [r, g, b, a] = image
        .get_pixel(0, 0)
        .0
        .map(|channel| f32::from(channel) / 255.0);
    let over = |channel: f32| channel * a + (1.0 - a);
    Ok(vec![Effect::Color(Rgba([over(r), over(g), over(b), 1.0]))])
}
