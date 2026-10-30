//! Koharu's settings and secrets. The README is the design.

mod error;
mod secrets;

use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use atomicwrites::{AtomicFile, OverwriteBehavior};
use serde::{
    Serialize,
    de::{DeserializeOwned, IntoDeserializer as _},
};
use toml::de::DeTable;
use toml_edit::{DocumentMut, Item, TableLike};

pub use error::{Error, Result};
pub use secrecy::{ExposeSecret, SecretString};
pub use secrets::{delete_secret, secret, set_secret};

/// A component's settings: a table in `config.toml` that the component owns,
/// with the type's `Default` for whatever the file leaves out.
pub trait Section: Default + Serialize + DeserializeOwned {
    /// The table's name, such as `pipeline`.
    const NAME: &'static str;
}

/// The section as `config.toml` holds it now.
pub fn get<T: Section>() -> Result<T> {
    read(&home()?.join("config.toml"))
}

/// Runs `edit` on the section as the file holds it now and writes the keys
/// whose values it changed, before returning.
pub fn update<T: Section>(edit: impl FnOnce(&mut T)) -> Result<()> {
    write(&home()?.join("config.toml"), edit)
}

fn home() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".koharu"))
        .ok_or(Error::NoHome)
}

fn read<T: Section>(path: &Path) -> Result<T> {
    match read_text(path)? {
        Some(text) => section(path, &text),
        None => Ok(T::default()),
    }
}

fn write<T: Section>(path: &Path, edit: impl FnOnce(&mut T)) -> Result<()> {
    let _held = lock(path)?;
    let text = read_text(path)?.unwrap_or_default();
    let mut value: T = section(path, &text)?;
    let before = table(&value)?;
    edit(&mut value);
    let after = table(&value)?;
    if before == after {
        return Ok(());
    }

    let mut document: DocumentMut =
        text.parse()
            .map_err(|error: toml_edit::TomlError| Error::Toml {
                path: path.into(),
                message: error.to_string(),
            })?;
    let item = document.entry(T::NAME).or_insert_with(implicit_table);
    if !item.is_table_like() {
        *item = implicit_table();
    }
    if let Some(table) = item.as_table_like_mut() {
        edit_table(table, &before, &after);
    }
    save(path, document.to_string().as_bytes())
}

/// The file's bytes, or nothing when it doesn't exist; any other error fails,
/// so a file that can't be read is never taken for a missing one.
fn read_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Read {
            path: path.into(),
            source,
        }),
    }
}

fn read_text(path: &Path) -> Result<Option<String>> {
    read_file(path)?
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| Error::Toml {
                path: path.into(),
                message: "it isn't UTF-8".into(),
            })
        })
        .transpose()
}

fn section<T: Section>(path: &Path, text: &str) -> Result<T> {
    let root = DeTable::parse(text).map_err(|mut error| {
        error.set_input(Some(text));
        Error::Toml {
            path: path.into(),
            message: error.to_string(),
        }
    })?;
    let Some((_, value)) = root
        .into_inner()
        .into_iter()
        .find(|(key, _)| key.get_ref().as_ref() == T::NAME)
    else {
        return Ok(T::default());
    };
    serde_ignored::deserialize(value.into_deserializer(), |key| {
        warn_unknown(path, T::NAME, &key.to_string());
    })
    .map_err(|mut error: toml::de::Error| {
        error.set_input(Some(text));
        Error::Section {
            path: path.into(),
            section: T::NAME,
            message: error.to_string(),
        }
    })
}

/// Logs a key the section's type doesn't know, once per key in a process, as
/// `get` reads the file every time it's called.
fn warn_unknown(path: &Path, section: &str, key: &str) {
    static WARNED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    let key = format!("{section}.{key}");
    let first = WARNED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key.clone());
    if first {
        tracing::warn!("{} has a key Koharu doesn't use: `{key}`", path.display());
    }
}

fn table<T: Section>(value: &T) -> Result<toml::Table> {
    let error = |message: String| Error::Serialize {
        section: T::NAME,
        message,
    };
    match toml::Value::try_from(value).map_err(|error_| error(error_.to_string()))? {
        toml::Value::Table(table) => Ok(table),
        other => Err(error(format!("it's a {}", other.type_str()))),
    }
}

/// Writes the keys whose values differ between `before` and `after` into
/// `table`: keys `after` sets or changes, and keys only `before` has. Every
/// other key keeps its text, so a default nobody changed stays out of the
/// file and comments stay where they are.
fn edit_table(table: &mut dyn TableLike, before: &toml::Table, after: &toml::Table) {
    for (key, value) in after {
        let old = before.get(key);
        if old == Some(value) {
            continue;
        }
        if let toml::Value::Table(value) = value {
            let item = table.entry(key).or_insert(implicit_table());
            if !item.is_table_like() {
                *item = implicit_table();
            }
            let empty = toml::Table::new();
            let old = old.and_then(toml::Value::as_table).unwrap_or(&empty);
            if let Some(inner) = item.as_table_like_mut() {
                edit_table(inner, old, value);
            }
            continue;
        }
        let mut new = edit_value(value);
        if let Some(Item::Value(existing)) = table.get(key) {
            *new.decor_mut() = existing.decor().clone();
        }
        table.insert(key, Item::Value(new));
    }
    for key in before.keys().filter(|key| !after.contains_key(*key)) {
        table.remove(key);
    }
}

fn edit_value(value: &toml::Value) -> toml_edit::Value {
    match value {
        toml::Value::String(value) => value.as_str().into(),
        toml::Value::Integer(value) => (*value).into(),
        toml::Value::Float(value) => (*value).into(),
        toml::Value::Boolean(value) => (*value).into(),
        toml::Value::Datetime(value) => (*value).into(),
        toml::Value::Array(values) => values
            .iter()
            .map(edit_value)
            .collect::<toml_edit::Array>()
            .into(),
        toml::Value::Table(table) => table
            .iter()
            .map(|(key, value)| (key.as_str(), edit_value(value)))
            .collect::<toml_edit::InlineTable>()
            .into(),
    }
}

/// A table that shows no header of its own until it holds a value.
fn implicit_table() -> Item {
    let mut table = toml_edit::Table::new();
    table.set_implicit(true);
    Item::Table(table)
}

/// Holds `path` against every other save of it, in this process or another,
/// until the returned file is dropped. The lock is on a file beside it, as a
/// save replaces the file itself.
fn lock(path: &Path) -> Result<File> {
    let error = |source| Error::Write {
        path: path.into(),
        source,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(error)?;
    }
    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(".lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_file_name(name))
        .map_err(error)?;
    file.lock().map_err(error)?;
    Ok(file)
}

/// Replaces `path` with `contents` atomically.
fn save(path: &Path, contents: &[u8]) -> Result<()> {
    AtomicFile::new(path, OverwriteBehavior::AllowOverwrite)
        .write(|file| file.write_all(contents))
        .map_err(|source| Error::Write {
            path: path.into(),
            source: source.into(),
        })
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, Default, PartialEq, Deserialize, Serialize)]
    #[serde(default)]
    struct Pipeline {
        ocr: String,
        retries: u32,
        model: Option<String>,
        translation: Translation,
    }

    #[derive(Debug, Default, PartialEq, Deserialize, Serialize)]
    #[serde(default)]
    struct Translation {
        language: String,
        batch: u32,
    }

    impl Section for Pipeline {
        const NAME: &'static str = "pipeline";
    }

    #[test]
    fn update_writes_only_what_changed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            indoc! {r#"
                # hand-written
                [pipeline]
                ocr = "manga" # chosen
                model = "gemma"
                later = 1

                [pipeline.translation]
                language = "en"

                [other]
                kept = true
            "#},
        )
        .unwrap();

        write(&path, |pipeline: &mut Pipeline| {
            pipeline.ocr = "paddle".into();
            pipeline.model = None;
            pipeline.translation.batch = 4;
        })
        .unwrap();

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            indoc! {r#"
                # hand-written
                [pipeline]
                ocr = "paddle" # chosen
                later = 1

                [pipeline.translation]
                language = "en"
                batch = 4

                [other]
                kept = true
            "#},
        );
        let pipeline: Pipeline = read(&path).unwrap();
        assert_eq!(pipeline.retries, 0);
        assert_eq!(pipeline.translation.batch, 4);
    }

    #[test]
    fn a_value_of_the_wrong_type_fails_naming_its_key() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            indoc! {r#"
                [pipeline]
                retries = "three"
            "#},
        )
        .unwrap();

        let error = read::<Pipeline>(&path).unwrap_err().to_string();

        assert!(error.contains("[pipeline]"), "{error}");
        assert!(error.contains("retries"), "{error}");
        assert!(matches!(
            write(&path, |pipeline: &mut Pipeline| pipeline.retries = 3),
            Err(Error::Section { .. })
        ));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            indoc! {r#"
                [pipeline]
                retries = "three"
            "#}
        );
    }
}
