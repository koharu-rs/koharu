# koharu-config

Koharu's settings and secrets. Settings live in `~/.koharu/config.toml`, a
table per component; secrets, the API keys and sign-in tokens, live in
`~/.koharu/secrets.age`, encrypted with a key kept in the OS credential store.

```rust,ignore
// in koharu-pipeline: the section's type, its defaults, and its table
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PipelineConfig { /* … */ }

impl koharu_config::Section for PipelineConfig {
    const NAME: &'static str = "pipeline";
}

// anywhere
let pipeline: PipelineConfig = koharu_config::get()?;
koharu_config::update(|p: &mut PipelineConfig| p.ocr = Ocr::MangaOcr)?;

let key = koharu_config::secret("openai")?;            // Option<SecretString>
koharu_config::set_secret("openai", &key)?;
koharu_config::delete_secret("openai")?;
```

## Settings

- **Each component owns its section.** Its keys and defaults live with the
  code that uses them, so adding a setting touches one crate, and this one
  knows no section's keys.
- **The file is read on every call.** Nothing loads at startup, no copy goes
  stale, and there's nothing to subscribe to: an edit by hand applies at the
  next read.
- **Unknown keys are ignored; wrong values fail.** A key from another Koharu
  version or a typo shouldn't stop the app, but a value of the wrong type is
  an error rather than a silent default, so a setting never means something
  the user didn't write.
- **Saving edits the file rather than rewriting it.** The file is the user's
  too, so their comments, order, and unknown keys stay, and a default nobody
  changed isn't written down, so a later release can still change it.

## Secrets

- **Secrets stay out of settings,** so settings can be shown, logged, or sent
  to the frontend without exposing a key.
- **They're encrypted, never in plain text.** No environment file or plain
  config holds a key.
- **One encrypted file, one credential.** The OS credential store holds only
  the file's key. Storing each secret there instead would fail on Windows,
  which caps a credential at 2,560 bytes, shorter than the ChatGPT sign-in, and
  would mean a keyring prompt per secret on Linux.
- **Linux needs a Secret Service,** such as GNOME Keyring, KWallet, or
  KeePassXC. Without one, secrets can't be stored: the other ways to keep the
  key are a file beside the secrets, which encrypts nothing, or the kernel
  keyring, which forgets it at every reboot.
- **`age` does the encryption,** a small, widely used format with a maintained
  Rust crate, so Koharu designs no cryptography of its own.
- **A file that won't decrypt is never replaced,** so a lost or changed key
  can't silently wipe the user's secrets.
- **The calls block,** as the Secret Service may ask to unlock the keyring, so
  callers run them off the async threads.
