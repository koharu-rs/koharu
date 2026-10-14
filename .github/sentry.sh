#!/usr/bin/env bash
set -euo pipefail

# Upload symbols before stripping; Tauri bundles and signs afterward.
binary=target/release/koharu
case "$RUNNER_OS" in
  macOS)
    binary=target/aarch64-apple-darwin/release/koharu
    dsymutil "$binary"
    sentry-cli debug-files upload --include-sources --wait "$binary.dSYM"
    strip -S -x "$binary"
    ;;
  Windows)
    sentry-cli debug-files upload --include-sources --wait "$binary.exe" "$binary.pdb"
    # Debug information is separate in the PDB, which Tauri does not bundle.
    ;;
  Linux)
    # Upload the full ELF so Sentry also has the unwind information.
    sentry-cli debug-files upload --include-sources --wait "$binary"
    llvm-strip --strip-debug "$binary"
    ;;
esac
