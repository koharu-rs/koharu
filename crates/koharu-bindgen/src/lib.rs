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
        let file = syn::parse_file(&source).context("couldn't parse bindgen's output")?;
        let source = rewrite::rewrite(file, None).context("couldn't rewrite bindgen's output")?;
        let path = path.as_ref();
        fs::write(path, source).with_context(|| format!("couldn't write {}", path.display()))
    }
}

/// Rewrites bindings a crate declares in Rust, rather than in a C header,
/// into `path`: `root` and the module files it declares, with every `extern`
/// block's functions resolved by `load` as [`Bindings`] resolves a header's.
pub fn rewrite(root: impl AsRef<Path>, path: impl AsRef<Path>) -> Result<()> {
    let root = root.as_ref();
    let source =
        fs::read_to_string(root).with_context(|| format!("couldn't read {}", root.display()))?;
    let file =
        syn::parse_file(&source).with_context(|| format!("couldn't parse {}", root.display()))?;
    let source = rewrite::rewrite(file, root.parent())
        .with_context(|| format!("couldn't rewrite {}", root.display()))?;
    let path = path.as_ref();
    fs::write(path, source).with_context(|| format!("couldn't write {}", path.display()))
}
