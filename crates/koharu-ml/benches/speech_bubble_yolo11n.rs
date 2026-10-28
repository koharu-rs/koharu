use std::{hint::black_box, path::PathBuf, time::Duration};

use anyhow::Result;
use criterion::Criterion;
use koharu_ml::speech_bubble_yolo11n::Yolo11nSpeechBubbleSegmenter;

#[tokio::main]
async fn main() -> Result<()> {
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("fixtures")
        .join("object_detection")
        .join("1.jpg");

    let runtime = koharu_runtime::Runtime::start().await?;
    let image = image::open(&input)?;
    let model = Yolo11nSpeechBubbleSegmenter::load(runtime.device()).await?;

    model.inference(&image)?;

    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
        .configure_from_args();

    criterion.bench_function("speech_bubble_yolo11n/inference/770x1080", |bencher| {
        bencher.iter(|| {
            let result = model
                .inference(black_box(&image))
                .expect("YOLO11n speech bubble inference failed");
            black_box(result);
        });
    });
    criterion.final_summary();
    Ok(())
}
