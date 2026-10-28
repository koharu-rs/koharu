//! The build step behind Koharu's `-sys` crates. The README is the design.

mod rewrite;

use std::{fs, io::Write, path::Path};

use anyhow::{Context, Result};

/// Bindings for a C header whose functions are resolved from libraries
/// loaded at run time.
pub struct Bindings {
    builder: bindgen::Builder,
}

impl Bindings {
    pub fn new(header: impl AsRef<Path>) -> Self {
        Self {
            builder: bindgen::Builder::default().header(header.as_ref().display().to_string()),
        }
    }

    /// Sets bindgen's own options: allowlists, clang arguments, derives.
    pub fn configure(
        mut self,
        configure: impl FnOnce(bindgen::Builder) -> bindgen::Builder,
    ) -> Self {
        self.builder = configure(self.builder);
        self
    }

    pub fn write(self, path: impl AsRef<Path>) -> Result<()> {
        let bindings = self
            .builder
            // The rewrite formats its own output.
            .formatter(bindgen::Formatter::None)
            .generate()
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("bindgen failed")?;
        let mut source = Vec::new();
        bindings.write(Box::new(&mut source) as Box<dyn Write>)?;
        let source = String::from_utf8(source).context("bindgen wrote non-UTF-8 Rust")?;
        let source = rewrite::rewrite(&source).context("couldn't rewrite bindgen's output")?;
        let path = path.as_ref();
        fs::write(path, source).with_context(|| format!("couldn't write {}", path.display()))
    }
}
