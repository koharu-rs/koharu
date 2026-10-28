//! Secrets live in `secrets.age`: a JSON map from name to value, encrypted with
//! `age` to an X25519 key that is the only thing kept in the OS credential
//! store, so a value of any length fits.

use std::{collections::BTreeMap, path::Path, str::FromStr};

use age::x25519::Identity;
use secrecy::{ExposeSecret, SecretString};

use crate::{Error, Result, home, lock, read_file, save};

/// The credential holding the file's key.
const SERVICE: &str = "koharu";
const ACCOUNT: &str = "secrets";

type Secrets = BTreeMap<String, String>;

/// The secret stored under `name`, or nothing when there's none.
pub fn secret(name: &str) -> Result<Option<SecretString>> {
    let path = home()?.join("secrets.age");
    Ok(open(&path)?
        .and_then(|(_, mut secrets)| secrets.remove(name))
        .map(SecretString::from))
}

/// Stores `value` under `name`, replacing what was there.
pub fn set_secret(name: &str, value: &SecretString) -> Result<()> {
    if value.expose_secret().is_empty() {
        return Err(Error::EmptySecret(name.into()));
    }
    edit(|secrets| {
        secrets.insert(name.into(), value.expose_secret().into());
    })
}

/// Forgets the secret under `name`; forgetting one that isn't stored succeeds.
pub fn delete_secret(name: &str) -> Result<()> {
    edit(|secrets| {
        secrets.remove(name);
    })
}

fn edit(change: impl FnOnce(&mut Secrets)) -> Result<()> {
    let path = home()?.join("secrets.age");
    let _held = lock(&path)?;
    let (key, mut secrets) = match open(&path)? {
        Some((key, secrets)) => (Some(key), secrets),
        None => (None, Secrets::new()),
    };
    let before = secrets.clone();
    change(&mut secrets);
    if secrets == before {
        return Ok(());
    }
    // The key is made with the first secret; an existing file always comes
    // with its key, or `open` failed.
    let key = match key {
        Some(key) => key,
        None => {
            let key = Identity::generate();
            entry()?.set_password(key.to_string().expose_secret())?;
            key
        }
    };
    let plaintext = serde_json::to_vec(&secrets).map_err(|error| secrets_error(&path, error))?;
    let ciphertext =
        age::encrypt(&key.to_public(), &plaintext).map_err(|error| secrets_error(&path, error))?;
    save(&path, &ciphertext)
}

/// The file's key and secrets, or nothing when there's no file. A file whose
/// key is missing, or that doesn't decrypt, fails rather than reading as
/// empty, so a save never replaces it.
fn open(path: &Path) -> Result<Option<(Identity, Secrets)>> {
    let Some(ciphertext) = read_file(path)? else {
        return Ok(None);
    };
    let key = match entry()?.get_password() {
        Ok(key) => Identity::from_str(&key).map_err(|error| secrets_error(path, error))?,
        Err(keyring::Error::NoEntry) => {
            return Err(secrets_error(
                path,
                "its key isn't in the OS credential store",
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let plaintext = age::decrypt(&key, &ciphertext).map_err(|error| secrets_error(path, error))?;
    let secrets = serde_json::from_slice(&plaintext).map_err(|error| secrets_error(path, error))?;
    Ok(Some((key, secrets)))
}

fn entry() -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(SERVICE, ACCOUNT)?)
}

fn secrets_error(path: &Path, reason: impl ToString) -> Error {
    Error::Secrets {
        path: path.into(),
        reason: reason.to_string(),
    }
}
