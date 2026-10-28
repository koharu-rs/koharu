use std::{hint::black_box, path::PathBuf, time::Duration};

use anyhow::Result;
use criterion::Criterion;
use koharu_ml::lama::{InpaintRequest, LaMa};

#[tokio::main]
async fn main() -> Result<()> {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("fixtures")
        .join("inpaint");
    let image_path = fixtures.join("image_4k.jpg");
    let mask_path = fixtures.join("mask_4k.png");

    let image = image::open(&image_path)?;
    let mask = image::open(&mask_path)?.to_luma8();

    let runtime = koharu_runtime::Runtime::start().await?;
    let model = LaMa::load(runtime.device()).await?;
    let config = InpaintRequest::default();

    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
        .configure_from_args();

    criterion.bench_function("lama/inference/3840x2074", |bencher| {
        bencher.iter(|| {
            let output = model
                .inference(black_box(&image), black_box(&mask), black_box(&config))
                .expect("LaMa inference failed");
            black_box(output);
        });
    });
    criterion.final_summary();

    Ok(())
}
