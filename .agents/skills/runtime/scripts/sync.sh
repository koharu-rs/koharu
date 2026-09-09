#!/usr/bin/env bash

source_tag() {
    local repository=$1
    local release=$2
    local source=${release%.*}

    if [[ "$source" == "$release" ]] || ! gh api "repos/$repository/releases/tags/$source" --silent 2>/dev/null; then
        source=$release
    fi
    printf '%s' "$source"
}

llama_release=$(gh api 'repos/koharu-rs/llama/releases?per_page=1' --jq '.[0].tag_name')
diffusion_release=$(gh api 'repos/koharu-rs/diffusion/releases?per_page=1' --jq '.[0].tag_name')
llama=$(source_tag koharu-rs/llama "$llama_release")
diffusion=$(source_tag koharu-rs/diffusion "$diffusion_release")

while read -r source target; do
    curl -sL "https://raw.githubusercontent.com/ggml-org/llama.cpp/$llama/$source" -o "$target"
done <<'EOF'
include/llama.h crates/koharu-llama-sys/include/llama.h
ggml/include/gguf.h crates/koharu-llama-sys/include/gguf.h
ggml/include/ggml.h crates/koharu-llama-sys/include/ggml.h
ggml/include/ggml-alloc.h crates/koharu-llama-sys/include/ggml-alloc.h
ggml/include/ggml-backend.h crates/koharu-llama-sys/include/ggml-backend.h
ggml/include/ggml-cpu.h crates/koharu-llama-sys/include/ggml-cpu.h
ggml/include/ggml-opt.h crates/koharu-llama-sys/include/ggml-opt.h
tools/mtmd/mtmd.h crates/koharu-llama-sys/include/mtmd.h
tools/mtmd/mtmd-helper.h crates/koharu-llama-sys/include/mtmd-helper.h
EOF

curl -sL \
    "https://raw.githubusercontent.com/leejet/stable-diffusion.cpp/$diffusion/include/stable-diffusion.h" \
    -o crates/koharu-diffusion-sys/include/stable-diffusion.h

printf 'llama.cpp: %s\nstable-diffusion.cpp: %s\n' "$llama_release" "$diffusion_release"
