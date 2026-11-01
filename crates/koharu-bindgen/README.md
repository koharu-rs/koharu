# koharu-bindgen

The build step behind Koharu's `-sys` crates. It runs bindgen over a crate's
headers and turns each C function into one of the same name and signature that
calls into a library loaded at run time, so one Koharu binary runs whichever
build of libtorch, llama.cpp, and stable-diffusion.cpp fits the GPU.
