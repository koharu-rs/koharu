use std::{hint::black_box, path::PathBuf, time::Duration};

use anyhow::Result;
use criterion::Criterion;
use koharu_ml::pp_ocr_v6::rec::PPOCRV6MediumRec;

#[tokio::main]
async fn main() -> Result<()> {
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("fixtures")
        .join("ocr")
        .join("title.png");

    let runtime = koharu_runtime::Runtime::start().await?;

    let image = image::open(&input)?;
    let model = PPOCRV6MediumRec::load(runtime.device()).await?;

    black_box(model.inference(&image)?);

    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
        .configure_from_args();

    let benchmark = format!(
        "pp_ocr_v6/rec/inference/{}x{}",
        image.width(),
        image.height()
    );
    criterion.bench_function(&benchmark, |bencher| {
        bencher.iter(|| {
            let recognition = model
                .inference(black_box(&image))
                .expect("PP-OCRv6 recognizer inference failed");
            black_box(recognition);
        });
    });
    criterion.final_summary();

    Ok(())
}
