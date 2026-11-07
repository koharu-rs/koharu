# koharu-config

Settings and secrets, in two files under `~/.koharu`:

| File | Holds | Format |
|---|---|---|
| `config.toml` | Settings, one table per component | TOML, meant to be edited by hand |
| `secrets.age` | API keys and sign-in tokens | A JSON map encrypted with `age` |

## Settings

A component owns its table by implementing `Section`. Its defaults are what an empty table
deserializes to, so the type needs `#[serde(default)]`. Without it, `get` fails whatever
the file holds.

```rust
#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Pipeline {
    ocr: String,
}

impl koharu_config::Section for Pipeline {
    const NAME: &'static str = "pipeline";
}

let pipeline: Pipeline = koharu_config::get()?;
koharu_config::update(|pipeline: &mut Pipeline| pipeline.ocr = "manga-ocr-ar".into())?;
```

- **`get`** reads the file on every call, with no cache or subscriptions. A missing file
  or table reads as an empty table.
- **`update`** takes a lock on `.config.toml.lock`, which holds across processes, and runs
  the edit on what the file holds now. It writes only what changed, so comments and other
  tables stay as they are, and no change means no write. The file is replaced atomically.
- **Overrides only.** A key of the section set back to its default leaves the file, so
  resetting is assigning the default inside `update`, and that user follows later changes
  to it. Keys inside a nested table stay, as a missing one falls back to that type's own
  defaults.
- **Reads back.** `update` parses what it's about to save, and fails without saving unless
  `get` would return exactly what the edit left. A nested table new to the file is written
  whole for the same reason.
- **Unknown keys** log a warning once per key per process, and stay in the file on save.
- **Invalid files fail.** Bad TOML, a wrong type, or an unreadable file is an error, and
  is never treated as a missing file or replaced by defaults.

## Secrets

| Function | What it does |
|---|---|
| `secrets::get(name)` | The value as a `SecretString`, or `None` if it isn't stored |
| `secrets::set(name, value)` | Stores the value. An empty value is an error |
| `secrets::delete(name)` | Removes it. Removing one that isn't stored succeeds |

`secrets.age` is encrypted to an X25519 key, which is the only thing in the OS credential
store, under service `koharu` and account `secrets`. So a value can be any length. The key
is made with the first secret. These calls can block on the file lock or a credential store
prompt, so async code should run them on a blocking thread.

| Situation | What happens |
|---|---|
| The key is gone, unusable, or was made for another file | The file can never be read again, so it's deleted with a logged warning, and the next secret starts a new key and file |
| The credential store can't answer, such as when it's locked | `Error::CredentialStore`, and the file stays |
| There's no credential store | `Error::NoCredentialStore`, unless there's no file to open. On Linux the store is a Secret Service, such as GNOME Keyring, KWallet, or KeePassXC. There's no environment variable or plain-text fallback |
| The file is damaged | `Error::SecretsUnreadable`, and the file stays |

Saves use the same kind of lock and atomic replace as settings.
