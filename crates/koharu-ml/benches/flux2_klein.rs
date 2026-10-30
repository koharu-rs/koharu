use std::{hint::black_box, path::PathBuf, time::Duration};

use anyhow::Result;
use criterion::Criterion;
use koharu_ml::flux2_klein::{Flux2Klein, Flux2KleinInpaintOptions};

#[tokio::main]
async fn main() -> Result<()> {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("fixtures")
        .join("inpaint");
    let image = image::open(fixtures.join("image_4k.jpg"))?;
    let mask = image::open(fixtures.join("mask_4k.png"))?;
    let options = Flux2KleinInpaintOptions::default();

    let device = koharu_runtime::device().await?;
    let model = Flux2Klein::load(device, device.memory().free).await?;

    let output = model.inpaint("Remove the masked content.", &image, None, &mask, &options)?;
    let scale = (1_048_576.0 / (f64::from(image.width()) * f64::from(image.height()))).sqrt();
    let expected_width = ((f64::from(image.width()) * scale).floor() as u32 / 16) * 16;
    let expected_height = ((f64::from(image.height()) * scale).floor() as u32 / 16) * 16;
    assert_eq!(output.width(), expected_width);
    assert_eq!(output.height(), expected_height);
    black_box(output);

    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
        .configure_from_args();

    criterion.bench_function(
        "flux2_klein/inpaint/3840x2074/max_pixels_1048576",
        |bencher| {
            bencher.iter(|| {
                let output = model
                    .inpaint(
                        black_box("Remove the masked content."),
                        black_box(&image),
                        None,
                        black_box(&mask),
                        black_box(&options),
                    )
                    .expect("FLUX.2 Klein inference failed");
                black_box(output);
            });
        },
    );
    criterion.final_summary();

    Ok(())
}
