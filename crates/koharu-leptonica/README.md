# koharu-leptonica

Rust ports of the Leptonica 1.87.0 algorithms no Rust crate covers: skew
finding, stroke width, colour segmentation, and Otsu and Sauvola
binarization. Each keeps Leptonica's integer and float arithmetic, so it gives
Leptonica's result on the same image, and works on `image` buffers.
