#!/usr/bin/env bash
# Regenerates koharu-torch/src/wrappers/tensor_generated.rs with tch-rs's
# generator at its pinned commit and Koharu's patch to it. It runs the OCaml
# generator in Docker.
#
#   crates/koharu-torch/gen/generate.sh
set -euo pipefail

TCH_COMMIT=4227b89b72059b0651ff83a38637693574e0a2d6
DECLARATIONS=third_party/pytorch/Declarations-v2.13.0.yaml

gen=$(cd "$(dirname "$0")" && pwd)
crates=$(dirname "$(dirname "$gen")")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

git clone --quiet https://github.com/LaurentMazare/tch-rs.git "$work/tch-rs"
git -C "$work/tch-rs" checkout --quiet "$TCH_COMMIT"
git -C "$work/tch-rs" apply "$gen/gen.patch"
mkdir "$work/out"
cp "$gen/ops.txt" "$work/out/ops.txt"

docker run --rm -v "$work:/work" -w /work/tch-rs ocaml/opam:debian-12-ocaml-5.2 bash -lc "
  opam install -y dune base stdio yaml >/dev/null &&
  opam exec -- dune exec --profile release gen/gen.exe -- \
    $DECLARATIONS /work/out/ops.txt /work/out/tensor_generated.rs"

cp "$work/out/tensor_generated.rs" "$crates/koharu-torch/src/wrappers/"
