# koharu-runtime

Gets Koharu ready to run models on this machine. It finds the best GPU
(CUDA, ROCm, Metal, then Vulkan), installs the native libraries that run on it
(libtorch, llama.cpp, stable-diffusion.cpp, and the CUDA or ROCm libraries
they need), and downloads model files from Hugging Face, reporting each
download's progress. libtorch runs on the CPU where it has no build for the
GPU, as on Vulkan. Everything is pinned to a version and kept in one store, so
after the first install nothing goes over the network.
