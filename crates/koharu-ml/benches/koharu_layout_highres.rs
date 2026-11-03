use std::{hint::black_box, path::PathBuf, time::Duration};

use anyhow::Result;
use criterion::Criterion;
use koharu_ml::koharu_layout_highres::{KoharuLayoutHighres, Options};

#[tokio::main]
async fn main() -> Result<()> {
    let input =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/fixtures/object_detection/1.jpg");

    let device = koharu_runtime::device().await?;
    let image = image::open(input)?;
    let model = KoharuLayoutHighres::load(device).await?;

    let warmup = model.inference(&image, &Options::default())?;
    black_box(warmup);

    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
        .configure_from_args();
    criterion.bench_function("koharu_layout_highres/inference", |bencher| {
        bencher.iter(|| {
            let detections = model
                .inference(black_box(&image), black_box(&Options::default()))
                .expect("Koharu Layout HighRes inference failed");
            black_box(detections);
        });
    });
    criterion.final_summary();
    Ok(())
}
