use std::{hint::black_box, path::PathBuf, time::Duration};

use anyhow::Result;
use criterion::Criterion;
use koharu_ml::koharu_layout_rfdetr_seg_2xl::{InputFit, KoharuLayoutRFDetrSeg2XL};

#[tokio::main]
async fn main() -> Result<()> {
    let input =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/fixtures/object_detection/1.jpg");

    koharu_ml::init().await?;
    let image = image::open(input)?;
    let model = KoharuLayoutRFDetrSeg2XL::load(koharu_ml::Device::default()).await?;
    // Bench the shipped default, which is also the configuration an A/B run
    // compares against.
    let input_fit = InputFit::default();

    let warmup = model.inference(&image, input_fit)?;
    black_box(warmup);

    let mut criterion = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
        .configure_from_args();
    criterion.bench_function("koharu_layout_rfdetr_seg_2xl/inference", |bencher| {
        bencher.iter(|| {
            let detections = model
                .inference(black_box(&image), input_fit)
                .expect("KoharuLayout RF-DETR inference failed");
            black_box(detections);
        });
    });
    criterion.final_summary();
    Ok(())
}
