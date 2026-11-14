//! Secrets live in `secrets.age`: a JSON map from name to value, encrypted with
//! `age` to an X25519 key that is the only thing kept in the OS credential
//! store, so a value of any length fits.
//!
//! These calls take a file lock and ask the OS credential store, either of which
//! can block, so async code should run them on a blocking thread.

use std::{collections::BTreeMap, path::Path, str::FromStr};

use age::x25519::Identity;

use crate::{Error, Result, home, lock, read_file, save};

pub use secrecy::{ExposeSecret, SecretString};

/// The credential holding the file's key.
const SERVICE: &str = "koharu";
const ACCOUNT: &str = "secrets";

type Secrets = BTreeMap<String, String>;

/// The secret stored under `name`, or nothing when there's none.
pub fn get(name: &str) -> Result<Option<SecretString>> {
    read(&path()?, &CredentialStore, name)
}

/// Stores `value` under `name`, replacing what was there.
pub fn set(name: &str, value: &SecretString) -> Result<()> {
    store(&path()?, &CredentialStore, name, value)
}

/// Forgets the secret under `name`; forgetting one that isn't stored succeeds.
pub fn delete(name: &str) -> Result<()> {
    edit(&path()?, &CredentialStore, |secrets| {
        secrets.remove(name);
    })
}

fn path() -> Result<std::path::PathBuf> {
    Ok(home()?.join("secrets.age"))
}

/// Where the file's key lives, apart from the file so tests needn't touch the
/// OS credential store.
trait Keys {
    /// The key, or nothing when it's gone or no longer usable. A store that
    /// can't answer is an error, so a locked store never reads as a lost key.
    fn get(&self) -> Result<Option<Identity>>;
    fn set(&self, key: &Identity) -> Result<()>;
}

struct CredentialStore;

impl Keys for CredentialStore {
    fn get(&self) -> Result<Option<Identity>> {
        match entry()?.get_password() {
            Ok(key) => Ok(Identity::from_str(&key).ok()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(Error::CredentialStore(error.to_string())),
        }
    }

    fn set(&self, key: &Identity) -> Result<()> {
        entry()?
            .set_password(key.to_string().expose_secret())
            .map_err(|error| Error::CredentialStore(error.to_string()))
    }
}

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|error| {
        match (error, keyring::Entry::store_status()) {
            // keyring sets its store up once, answering every later call
            // with `NoDefaultStore` and keeping why in `store_status`.
            (keyring::Error::NoDefaultStore, Err(cause)) => {
                Error::NoCredentialStore(cause.to_string())
            }
            (error, _) => Error::CredentialStore(error.to_string()),
        }
    })
}

fn read(path: &Path, keys: &impl Keys, name: &str) -> Result<Option<SecretString>> {
    // With no file, there's no key to ask for, and no store needed.
    if !fs_err::exists(path)? {
        return Ok(None);
    }
    let _held = lock(path)?;
    Ok(open(path, keys)?
        .and_then(|(_, mut secrets)| secrets.remove(name))
        .map(SecretString::from))
}

fn store(path: &Path, keys: &impl Keys, name: &str, value: &SecretString) -> Result<()> {
    if value.expose_secret().is_empty() {
        return Err(Error::EmptySecret(name.into()));
    }
    edit(path, keys, |secrets| {
        secrets.insert(name.into(), value.expose_secret().into());
    })
}

fn edit(path: &Path, keys: &impl Keys, change: impl FnOnce(&mut Secrets)) -> Result<()> {
    let _held = lock(path)?;
    let (key, mut secrets) = match open(path, keys)? {
        Some((key, secrets)) => (Some(key), secrets),
        None => (None, Secrets::new()),
    };
    let before = secrets.clone();
    change(&mut secrets);
    if secrets == before {
        return Ok(());
    }
    // The key is made with the first secret, in the file's first save or after
    // `open` found its key gone.
    let key = match key {
        Some(key) => key,
        None => {
            let key = Identity::generate();
            keys.set(&key)?;
            key
        }
    };
    let plaintext = serde_json::to_vec(&secrets).map_err(|error| unreadable(path, error))?;
    let ciphertext =
        age::encrypt(&key.to_public(), &plaintext).map_err(|error| unreadable(path, error))?;
    save(path, &ciphertext)
}

/// The file's key and secrets under the lock, or nothing when there's no file
/// or its key is gone. Without its key the file can never be read again, so it
/// goes too, and the next secret starts a new one. A store that can't answer,
/// or a damaged file, is an error and the file stays.
fn open(path: &Path, keys: &impl Keys) -> Result<Option<(Identity, Secrets)>> {
    let Some(ciphertext) = read_file(path)? else {
        return Ok(None);
    };
    let opened = match keys.get()? {
        Some(key) => match age::decrypt(&key, &ciphertext) {
            Ok(plaintext) => Some((key, plaintext)),
            // The key in the store was made for another file.
            Err(age::DecryptError::NoMatchingKeys) => None,
            Err(error) => return Err(unreadable(path, error)),
        },
        None => None,
    };
    let Some((key, plaintext)) = opened else {
        tracing::warn!(
            "{}'s key is gone, so its secrets are forgotten",
            path.display()
        );
        fs_err::remove_file(path)?;
        return Ok(None);
    };
    let secrets = serde_json::from_slice(&plaintext).map_err(|error| unreadable(path, error))?;
    Ok(Some((key, secrets)))
}

fn unreadable(path: &Path, reason: impl ToString) -> Error {
    Error::SecretsUnreadable {
        path: path.into(),
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::PathBuf};

    use tempfile::TempDir;

    use super::*;

    #[derive(Default)]
    struct Fake(RefCell<Option<Identity>>);

    impl Keys for Fake {
        fn get(&self) -> Result<Option<Identity>> {
            Ok(self.0.borrow().clone())
        }

        fn set(&self, key: &Identity) -> Result<()> {
            *self.0.borrow_mut() = Some(key.clone());
            Ok(())
        }
    }

    /// A store that's there but can't answer, such as a locked keyring.
    struct Locked;

    impl Keys for Locked {
        fn get(&self) -> Result<Option<Identity>> {
            Err(Error::CredentialStore("locked".into()))
        }

        fn set(&self, _: &Identity) -> Result<()> {
            Err(Error::CredentialStore("locked".into()))
        }
    }

    fn file() -> (TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("secrets.age");
        (directory, path)
    }

    fn value(text: &str) -> SecretString {
        SecretString::from(text)
    }

    fn stored(path: &Path, keys: &impl Keys, name: &str) -> Option<String> {
        read(path, keys, name)
            .unwrap()
            .map(|secret| secret.expose_secret().to_owned())
    }

    #[test]
    fn a_secret_is_encrypted_and_reads_back() {
        let (_directory, path) = file();
        let keys = Fake::default();

        store(&path, &keys, "openai", &value("sk-1")).unwrap();

        assert!(
            !fs_err::read(&path)
                .unwrap()
                .windows(4)
                .any(|w| w == b"sk-1")
        );
        assert_eq!(stored(&path, &keys, "openai").as_deref(), Some("sk-1"));
        edit(&path, &keys, |secrets| {
            secrets.remove("openai");
        })
        .unwrap();
        assert_eq!(stored(&path, &keys, "openai"), None);
    }

    #[test]
    fn a_lost_key_forgets_the_secrets_and_starts_over() {
        let (_directory, path) = file();
        let keys = Fake::default();
        store(&path, &keys, "openai", &value("sk-1")).unwrap();

        *keys.0.borrow_mut() = None;

        assert_eq!(stored(&path, &keys, "openai"), None);
        assert!(!path.exists());
        store(&path, &keys, "claude", &value("sk-2")).unwrap();
        assert_eq!(stored(&path, &keys, "claude").as_deref(), Some("sk-2"));
    }

    #[test]
    fn a_key_made_for_another_file_counts_as_lost() {
        let (_directory, path) = file();
        let keys = Fake::default();
        store(&path, &keys, "openai", &value("sk-1")).unwrap();

        *keys.0.borrow_mut() = Some(Identity::generate());

        store(&path, &keys, "claude", &value("sk-2")).unwrap();
        assert_eq!(stored(&path, &keys, "openai"), None);
        assert_eq!(stored(&path, &keys, "claude").as_deref(), Some("sk-2"));
    }

    #[test]
    fn a_store_that_cant_answer_keeps_the_file() {
        let (_directory, path) = file();
        store(&path, &Fake::default(), "openai", &value("sk-1")).unwrap();
        let bytes = fs_err::read(&path).unwrap();

        assert!(matches!(
            read(&path, &Locked, "openai"),
            Err(Error::CredentialStore(_))
        ));
        assert!(matches!(
            store(&path, &Locked, "claude", &value("sk-2")),
            Err(Error::CredentialStore(_))
        ));

        assert_eq!(fs_err::read(&path).unwrap(), bytes);
    }

    #[test]
    fn a_damaged_file_fails_and_stays() {
        let (_directory, path) = file();
        let keys = Fake::default();
        store(&path, &keys, "openai", &value("sk-1")).unwrap();
        fs_err::write(&path, b"not an age file").unwrap();

        assert!(matches!(
            read(&path, &keys, "openai"),
            Err(Error::SecretsUnreadable { .. })
        ));
        assert!(matches!(
            store(&path, &keys, "claude", &value("sk-2")),
            Err(Error::SecretsUnreadable { .. })
        ));

        assert_eq!(fs_err::read(&path).unwrap(), b"not an age file");
    }

    #[test]
    fn an_empty_secret_is_refused() {
        let (_directory, path) = file();

        assert!(matches!(
            store(&path, &Fake::default(), "openai", &value("")),
            Err(Error::EmptySecret(_))
        ));
        assert!(!path.exists());
    }

    #[test]
    fn no_file_needs_no_store() {
        let (_directory, path) = file();

        assert_eq!(stored(&path, &Locked, "openai"), None);
    }
}
