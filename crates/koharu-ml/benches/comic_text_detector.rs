use std::{hint::black_box, path::PathBuf, time::Duration};

use anyhow::Result;
use criterion::Criterion;
use koharu_ml::comic_text_detector::ComicTextDetector;

#[tokio::main]
async fn main() -> Result<()> {
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("fixtures")
        .join("object_detection")
        .join("1.jpg");

    let runtime = koharu_runtime::Runtime::start().await?;

    let image = image::open(&input)?;
    let model = ComicTextDetector::load(runtime.device()).await?;

    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
        .configure_from_args();

    criterion.bench_function("comic_text_detector/inference/770x1080", |bencher| {
        bencher.iter(|| {
            let detection = model
                .inference(black_box(&image))
                .expect("Comic Text Detector inference failed");
            black_box(detection);
        });
    });
    criterion.final_summary();

    Ok(())
}
