//! Koharu's settings and secrets.

mod error;
pub mod secrets;

use std::{
    collections::BTreeSet,
    ffi::OsString,
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use atomicwrites::{AtomicFile, OverwriteBehavior};
use fs_err::{File, OpenOptions};
use serde::{
    Serialize,
    de::{DeserializeOwned, IntoDeserializer as _},
};
use toml::de::DeTable;
use toml_edit::{DocumentMut, Item, TableLike};

pub use error::{Error, Result};

/// A component's settings: a table in `config.toml` that the component owns.
/// Its defaults are what an empty table deserializes to, so a type needs
/// `#[serde(default)]`; without it, `get` fails whatever the file holds.
pub trait Section: Serialize + DeserializeOwned {
    /// The table's name, such as `pipeline`.
    const NAME: &'static str;
}

/// The section as `config.toml` holds it now.
pub fn get<T: Section>() -> Result<T> {
    read(&home()?.join("config.toml"))
}

/// Runs `edit` on the section as the file holds it now and writes what it
/// changed, before returning. `get` then reads back exactly what `edit` left:
/// if the file couldn't hold that, this fails and the file stays as it was. A
/// key of the section set back to its default leaves the file.
pub fn update<T: Section>(edit: impl FnOnce(&mut T)) -> Result<()> {
    write(&home()?.join("config.toml"), edit)
}

fn home() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".koharu"))
        .ok_or(Error::NoHome)
}

fn read<T: Section>(path: &Path) -> Result<T> {
    section(path, &read_text(path)?.unwrap_or_default())
}

fn write<T: Section>(path: &Path, edit: impl FnOnce(&mut T)) -> Result<()> {
    let _held = lock(path)?;
    let text = read_text(path)?.unwrap_or_default();
    let mut value: T = section(path, &text)?;
    let before = table(&value);
    edit(&mut value);
    let after = table(&value);
    if before == after {
        return Ok(());
    }

    let defaults = table(&empty_section::<T>(path)?);
    let mut document: DocumentMut = text
        .parse()
        .map_err(|error: toml_edit::TomlError| invalid(path, error))?;
    let item = document.entry(T::NAME).or_insert_with(implicit_table);
    if !item.is_table_like() {
        *item = implicit_table();
    }
    if let Some(table) = item.as_table_like_mut() {
        edit_table(table, &before, &after, Some(&defaults));
    }
    let text = document.to_string();
    if table(&section::<T>(path, &text)?) != after {
        return Err(invalid(
            path,
            format!("[{}] wouldn't read back as it was set", T::NAME),
        ));
    }
    save(path, text.as_bytes())
}

fn invalid(path: &Path, message: impl ToString) -> Error {
    Error::Invalid {
        path: path.into(),
        message: message.to_string(),
    }
}

/// The file's bytes, or nothing when it doesn't exist; any other error fails,
/// so a file that can't be read is never taken for a missing one.
fn read_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs_err::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn read_text(path: &Path) -> Result<Option<String>> {
    read_file(path)?
        .map(|bytes| String::from_utf8(bytes).map_err(|_| invalid(path, "it isn't UTF-8")))
        .transpose()
}

fn section<T: Section>(path: &Path, text: &str) -> Result<T> {
    let root = DeTable::parse(text).map_err(|mut error| {
        error.set_input(Some(text));
        invalid(path, error)
    })?;
    let Some((_, value)) = root
        .into_inner()
        .into_iter()
        .find(|(key, _)| key.get_ref().as_ref() == T::NAME)
    else {
        return empty_section(path);
    };
    serde_ignored::deserialize(value.into_deserializer(), |key| {
        warn_unknown(path, T::NAME, &key.to_string());
    })
    .map_err(|mut error: toml::de::Error| {
        error.set_input(Some(text));
        invalid(path, format!("[{}]: {error}", T::NAME))
    })
}

/// The section with nothing in its table, which holds its defaults.
fn empty_section<T: Section>(path: &Path) -> Result<T> {
    toml::from_str("")
        .map_err(|error: toml::de::Error| invalid(path, format!("[{}]: {error}", T::NAME)))
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

/// The section as a table. A section that isn't one is a bug in its type, so
/// this panics rather than reporting it to whoever is changing a setting.
fn table<T: Section>(value: &T) -> toml::Table {
    match toml::Value::try_from(value) {
        Ok(toml::Value::Table(table)) => table,
        Ok(other) => panic!("[{}] is a {}, not a table", T::NAME, other.type_str()),
        Err(error) => panic!("[{}] can't be written as a table: {error}", T::NAME),
    }
}

/// Writes the keys whose values differ between `before` and `after` into
/// `table`: keys `after` sets or changes, and keys only `before` has. Every
/// other key keeps its text, so a default nobody changed stays out of the
/// file and comments stay where they are. With `defaults`, a key set back to
/// its default leaves the file. Only a section has them: inside a nested table
/// a missing key falls back to that type's own defaults, or fails.
fn edit_table(
    table: &mut dyn TableLike,
    before: &toml::Table,
    after: &toml::Table,
    defaults: Option<&toml::Table>,
) {
    for (key, value) in after {
        let old = before.get(key);
        if old == Some(value) {
            continue;
        }
        if defaults.is_some_and(|defaults| defaults.get(key) == Some(value)) {
            table.remove(key);
            continue;
        }
        if let toml::Value::Table(value) = value {
            // A table new to the file goes in whole: what it leaves out would
            // come from its type's defaults, not from what it was changed from.
            let held = table.get(key).is_some_and(Item::is_table_like);
            let item = table.entry(key).or_insert(implicit_table());
            if !item.is_table_like() {
                *item = implicit_table();
            }
            let empty = toml::Table::new();
            let old = old
                .and_then(toml::Value::as_table)
                .filter(|_| held)
                .unwrap_or(&empty);
            if let Some(inner) = item.as_table_like_mut() {
                edit_table(inner, old, value, None);
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
    if let Some(parent) = path.parent() {
        fs_err::create_dir_all(parent)?;
    }
    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(".lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_file_name(name))?;
    file.lock()?;
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
        fs_err::write(
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
            fs_err::read_to_string(&path).unwrap(),
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
        fs_err::write(
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
            Err(Error::Invalid { .. })
        ));
        assert_eq!(
            fs_err::read_to_string(&path).unwrap(),
            indoc! {r#"
                [pipeline]
                retries = "three"
            "#}
        );
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    struct NoDefaults {
        a: u32,
        b: u32,
    }

    impl Section for NoDefaults {
        const NAME: &'static str = "no-defaults";
    }

    #[test]
    fn a_section_without_defaults_fails_whatever_the_file_holds() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");

        assert!(matches!(
            read::<NoDefaults>(&path),
            Err(Error::Invalid { .. })
        ));
        fs_err::write(
            &path,
            indoc! {"
                [no-defaults]
                a = 1
            "},
        )
        .unwrap();
        assert!(matches!(
            read::<NoDefaults>(&path),
            Err(Error::Invalid { .. })
        ));
    }

    /// Required fields with its own `Default`, so a table holding only some of
    /// them doesn't deserialize.
    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    struct Selection {
        provider: String,
        model: String,
    }

    impl Default for Selection {
        fn default() -> Self {
            Self {
                provider: "local".into(),
                model: "gemma".into(),
            }
        }
    }

    #[derive(Debug, Default, PartialEq, Deserialize, Serialize)]
    #[serde(default)]
    struct Agent {
        model: Selection,
    }

    impl Section for Agent {
        const NAME: &'static str = "agent";
    }

    #[test]
    fn a_nested_table_new_to_the_file_goes_in_whole() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");

        write(&path, |agent: &mut Agent| {
            agent.model.model = "other".into();
        })
        .unwrap();

        assert_eq!(
            read::<Agent>(&path).unwrap().model,
            Selection {
                provider: "local".into(),
                model: "other".into(),
            }
        );
    }

    #[derive(Debug, Deserialize, PartialEq, Serialize)]
    #[serde(default)]
    struct Tuned {
        retries: u32,
        name: Option<String>,
    }

    impl Default for Tuned {
        fn default() -> Self {
            Self {
                retries: 5,
                name: Some("default".into()),
            }
        }
    }

    impl Section for Tuned {
        const NAME: &'static str = "tuned";
    }

    #[test]
    fn a_value_set_back_to_its_default_leaves_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs_err::write(
            &path,
            indoc! {"
                [tuned]
                retries = 9
                later = 1
            "},
        )
        .unwrap();

        write(&path, |tuned: &mut Tuned| tuned.retries = 5).unwrap();

        assert_eq!(
            fs_err::read_to_string(&path).unwrap(),
            indoc! {"
                [tuned]
                later = 1
            "}
        );
    }

    #[test]
    fn a_value_the_file_cant_hold_fails_and_keeps_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let text = indoc! {"
            [tuned]
            retries = 9
        "};
        fs_err::write(&path, text).unwrap();

        // TOML has no null, and a missing key reads as the default.
        assert!(matches!(
            write(&path, |tuned: &mut Tuned| tuned.name = None),
            Err(Error::Invalid { .. })
        ));

        assert_eq!(fs_err::read_to_string(&path).unwrap(), text);
    }
}
