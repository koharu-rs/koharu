use std::{hint::black_box, path::PathBuf, time::Duration};

use anyhow::Result;
use criterion::Criterion;
use koharu_ml::font_detector::FontDetector;

#[tokio::main]
async fn main() -> Result<()> {
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("fixtures")
        .join("ocr")
        .join("title.png");

    let runtime = koharu_runtime::Runtime::start().await?;
    let image = image::open(input)?;
    let model = FontDetector::load(runtime.device()).await?;

    // Load, decode, and warm up outside the measured loop.
    let _ = model.inference(std::slice::from_ref(&image), 5)?;

    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
        .configure_from_args();
    criterion.bench_function("font_detector/inference", |bencher| {
        bencher.iter(|| {
            let prediction = model
                .inference(black_box(std::slice::from_ref(&image)), black_box(5))
                .expect("YuzuMarker font detection failed");
            black_box(prediction);
        });
    });
    criterion.final_summary();
    Ok(())
}
