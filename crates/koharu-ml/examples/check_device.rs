use anyhow::{Result, ensure};
use koharu_torch::{Device, Kind, Tensor};

#[tokio::main]
async fn main() -> Result<()> {
    if let Some(store) = std::env::args_os().nth(1) {
        koharu_runtime::Store::configure(std::path::PathBuf::from(store))?;
    }
    println!(
        "Discovered: {:?}",
        koharu_runtime::Hardware::discover().device()
    );
    koharu_ml::init().await?;
    let selected = koharu_ml::device(false);
    println!("Initialized: {selected:?}");
    ensure!(
        selected.backend == koharu_runtime::Backend::Cuda,
        "CUDA was not selected"
    );
    let input = Tensor::f_ones([64, 64], (Kind::Float, Device::Cuda(selected.index)))?;
    let output = input
        .f_matmul(&input)?
        .f_sum(Kind::Float)?
        .f_double_value(&[])?;
    ensure!(
        output == 262144.0,
        "GPU matrix multiplication returned an unexpected result"
    );
    println!("CUDA matrix multiplication passed: {output}");
    let image = Tensor::f_ones([1, 3, 32, 32], (Kind::Float, Device::Cuda(selected.index)))?;
    let weights = Tensor::f_ones([4, 3, 3, 3], (Kind::Float, Device::Cuda(selected.index)))?;
    let convolution = image
        .f_conv2d(&weights, None::<&Tensor>, [1, 1], [0, 0], [1, 1], 1)?
        .f_sum(Kind::Float)?
        .f_double_value(&[])?;
    ensure!(
        convolution == 97200.0,
        "GPU convolution returned an unexpected result"
    );
    println!("CUDA convolution passed: {convolution}");
    Ok(())
}
