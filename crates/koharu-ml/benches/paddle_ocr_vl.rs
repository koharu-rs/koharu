use std::{hint::black_box, path::PathBuf, time::Duration};

use anyhow::Result;
use criterion::Criterion;
use koharu_ml::paddle_ocr_vl::{PaddleOCRVL, PaddleOCRVLTask};

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing::Level::WARN.into())
        .from_env_lossy();
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("fixtures")
        .join("ocr")
        .join("title.png");

    let runtime = koharu_runtime::Runtime::start().await?;
    let image = image::open(&input)?;
    let model = PaddleOCRVL::load(runtime.device()).await?;

    let warmup = model.inference(&image, PaddleOCRVLTask::Ocr)?;
    assert_eq!(warmup.text, "対策委員会です！");
    black_box(warmup);

    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
        .configure_from_args();
    let benchmark = format!(
        "paddle_ocr_vl/inference/ocr/{}x{}",
        image.width(),
        image.height()
    );

    criterion.bench_function(&benchmark, |bencher| {
        bencher.iter(|| {
            let result = model
                .inference(black_box(&image), black_box(PaddleOCRVLTask::Ocr))
                .expect("PaddleOCR-VL inference failed");
            black_box(result);
        });
    });
    criterion.final_summary();

    Ok(())
}
