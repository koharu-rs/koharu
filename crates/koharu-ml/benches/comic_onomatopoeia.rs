use std::{hint::black_box, path::PathBuf, time::Duration};

use anyhow::Result;
use criterion::Criterion;
use koharu_ml::comic_onomatopoeia::{ComicOnomatopoeiaDetector, ComicOnomatopoeiaRecognizer};

#[tokio::main]
async fn main() -> Result<()> {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/fixtures");
    let runtime = koharu_runtime::Runtime::start().await?;
    let detector_image = image::open(fixtures.join("object_detection/1.jpg"))?;
    let recognizer_image = image::open(fixtures.join("ocr/title.png"))?;
    let detector = ComicOnomatopoeiaDetector::load(runtime.device()).await?;
    let recognizer = ComicOnomatopoeiaRecognizer::load(runtime.device()).await?;

    // Resolution, decoding, model construction, and one real warm-up stay outside timing.
    black_box(detector.inference(black_box(&detector_image))?);
    black_box(recognizer.inference(black_box(&recognizer_image))?);

    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
        .configure_from_args();
    criterion.bench_function("comic_onomatopoeia/mtsv3/inference", |bencher| {
        bencher.iter(|| {
            let detections = detector
                .inference(black_box(&detector_image))
                .expect("COO MTSv3 inference failed");
            black_box(detections);
        });
    });
    criterion.bench_function("comic_onomatopoeia/trba_hardroi_2d/inference", |bencher| {
        bencher.iter(|| {
            let recognition = recognizer
                .inference(black_box(&recognizer_image))
                .expect("COO TRBA inference failed");
            black_box(recognition);
        });
    });
    criterion.final_summary();
    Ok(())
}
