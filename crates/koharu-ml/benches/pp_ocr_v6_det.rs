use std::{hint::black_box, path::PathBuf, time::Duration};

use anyhow::Result;
use criterion::Criterion;
use koharu_ml::pp_ocr_v6::det::PPOCRV6MediumDet;

#[tokio::main]
async fn main() -> Result<()> {
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("fixtures")
        .join("object_detection")
        .join("1.jpg");

    let runtime = koharu_runtime::Runtime::start().await?;

    let image = image::open(&input)?;
    let model = PPOCRV6MediumDet::load(runtime.device()).await?;

    black_box(model.inference(&image)?);

    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
        .configure_from_args();
    let benchmark = format!(
        "pp_ocr_v6/det/inference/{}x{}",
        image.width(),
        image.height()
    );

    criterion.bench_function(&benchmark, |bencher| {
        bencher.iter(|| {
            let detections = model
                .inference(black_box(&image))
                .expect("PP-OCRv6 detector inference failed");
            black_box(detections);
        });
    });
    criterion.final_summary();

    Ok(())
}
