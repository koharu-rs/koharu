mod rewrite;

use std::{fs, io::Write, path::Path};

use anyhow::{Context, Result};

pub use rewrite::{rewrite_bindings, rewrite_bindings_for_libraries};

/// Bindgen-backed generator that post-processes C function bindings into
/// top-level dynamic-loading adapters.
pub struct Generator {
    builder: bindgen::Builder,
    library_names: Vec<String>,
}

impl Generator {
    pub fn new(builder: bindgen::Builder, library_name: impl Into<String>) -> Self {
        Self {
            builder,
            library_names: vec![library_name.into()],
        }
    }

    pub fn from_header(header: impl AsRef<Path>, library_name: impl Into<String>) -> Self {
        Self::new(
            bindgen::Builder::default().header(header.as_ref().display().to_string()),
            library_name,
        )
    }

    pub fn with_bindgen(
        mut self,
        configure: impl FnOnce(bindgen::Builder) -> bindgen::Builder,
    ) -> Self {
        self.builder = configure(self.builder);
        self
    }

    pub fn with_libraries(
        mut self,
        library_names: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Self {
        self.library_names = library_names
            .into_iter()
            .map(|library_name| library_name.as_ref().to_owned())
            .collect();
        self
    }

    pub fn generate(self) -> Result<String> {
        let bindings = self
            .builder
            // The rewrite pass owns formatting with its matching syn/prettyplease pair.
            .formatter(bindgen::Formatter::None)
            .generate()
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("failed to run bindgen")?;
        let source = bindings_to_string(&bindings).context("failed to render bindgen output")?;

        let source = rewrite_bindings_for_libraries(&source, &self.library_names)
            .context("failed to rewrite bindgen output")?;
        Ok(source)
    }

    pub fn write_to_file(self, path: impl AsRef<Path>) -> Result<()> {
        let source = self.generate()?;
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(path, source).with_context(|| format!("failed to write {}", path.display()))
    }
}

fn bindings_to_string(bindings: &bindgen::Bindings) -> Result<String> {
    let mut bytes = Vec::new();
    bindings.write(Box::new(&mut bytes) as Box<dyn Write>)?;
    String::from_utf8(bytes).context("bindgen emitted non-UTF-8 Rust source")
}
