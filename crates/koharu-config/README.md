# koharu-config

`koharu-config` holds Koharu's settings and secrets. Settings live in
`~/.koharu/config.toml`, a table per component; secrets, the API keys and
sign-in tokens, live in `~/.koharu/secrets.age`, encrypted with a key kept in
the operating system's credential store. A component names its table once,
then reads and edits it with plain function calls: there's no macro, nothing to
load at startup, nothing to subscribe to, and no copy to keep in sync.

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

| Section | Owner |
|---|---|
| `pipeline` | `koharu-pipeline` |
| `providers`, `agent` | `koharu-llm` |
| `typesetting` | `koharu-renderer` |
| `http` | `koharu-runtime` |

| Rule | Evidence |
|---|---|
| A component owns its section: a serde type with its own defaults, and the table's name, which it gives by implementing `Section`. This crate knows no section's keys. | Codex reads each table into a typed struct [codex ConfigToml]; a VS Code extension contributes its own settings [VS Code contributions] |
| `get` reads the section from the file each time it's called, so nothing is loaded at startup and no value is kept in memory to go stale; an edit by hand applies at the next read. | Codex reads the file again for every config read [codex read]; VS Code hands out the current settings from `getConfiguration` [VS Code workspace] |
| A missing file or table reads as the type's `Default`, and a field its owner means to be optional says so with `#[serde(default)]`. | serde fills a field from `Default` only where it's marked [serde default] |
| A key the type doesn't know is ignored, with a warning in the log the first time it's read. A key it knows must hold a value of its type, or the read fails, naming the file, the table, and the key; so does a file that isn't TOML. | Cargo ignores a key it doesn't know and warns "unused manifest key", using `serde_ignored` [Cargo unused keys] [serde_ignored], as Codex's strict check does [codex strict] |
| Only a file that doesn't exist counts as missing; any other error reading it fails, so a file that can't be read is never replaced. | `ErrorKind::NotFound` [io NotFound] |
| `update` reads the section, runs the closure on it, and writes only the keys whose values changed: set, changed, or removed. Other keys, other tables, keys the type doesn't know, and comments stay as they were, and a default nobody changed isn't written into the file, so the owner can still change it. | Codex writes its config as edits to the keys they name, applied to a `toml_edit` document read just before [codex edits] [codex apply], and clears a key instead of writing its default [codex defaults]; `toml_edit` is "format-preserving" [toml_edit] |
| `update` saves before it returns and fails only when the disk does. The file is replaced atomically, and saves run one at a time, with a lock file held across processes. | Codex writes the file atomically [codex apply]; `atomicwrites` [atomicwrites]; `File::lock` [File lock] |

A component reads its settings where it uses them: the pipeline when a run
starts, `koharu-llm` when it sends a request, the renderer when it lays out a
page. Settings that shape something long-lived, such as the HTTP client's
timeouts, apply the next time it's built. Tests build section values
directly; only this crate's own tests touch a file.

## Secrets

| Name | Holds | Owner |
|---|---|---|
| a provider's id, such as `openai` or `claude` | its API key | `koharu-llm` |
| `chatgpt` | the ChatGPT sign-in, the one JSON record rig's token store saves: tokens, their expiry, and the account id | `koharu-llm` |

| Rule | Evidence |
|---|---|
| Secrets stay out of settings: no section holds a key, so a section can be shown in the preferences, logged, or sent to the frontend without exposing one. They share this crate but have their own calls. | VS Code keeps secrets in `SecretStorage`, "a storage utility for secrets", apart from its settings [VS Code secrets]; Codex keeps them behind its own `SecretsManager`, apart from `config.toml` [codex secrets] |
| All secrets live in one file, `secrets.age`, encrypted with `age` to an X25519 key that is the only thing kept in the credential store. A value of any length fits: Windows caps a credential at 2,560 bytes [Windows credential], and the ChatGPT record is longer. | Codex stores its sign-in this way on Windows: "the local encrypted secrets file, with the file key in the OS keyring" [codex auth storage] [codex secrets key]; `age`'s `encrypt` and `decrypt` [age] |
| The key is made on the first save. A file whose key is missing, or that doesn't decrypt, fails to open and is never replaced. | Codex makes a new key whenever none is stored [codex secrets key]; Koharu doesn't, so an existing file is never written over with a key that can't open it |
| The key lives in each platform's own credential store, chosen by `keyring`'s default mode: Keychain, Credential Manager, or the Secret Service, so it survives a reboot. | `keyring`'s default mode [keyring v1] |
| A save rewrites the file atomically under a lock file, as settings are saved. | Codex writes the secrets file atomically [codex secrets save] |
| A secret is `secrecy`'s `SecretString`, which shows only through `expose_secret` and can't be serialized. | `secrecy` [secrecy] |

`secret` returns nothing when no secret is stored, `delete_secret` succeeds
when there's nothing to delete, and `set_secret` refuses an empty value. The
secret calls block, and the Secret Service may ask to unlock the keyring, so
callers run them off the async threads.

## Changes from today

- `koharu-secrets` merges into this crate as a module.
- `get` and `update` over a type that implements `Section` replace `load(name)`
  handles: the read and write guards, revisions, change subscriptions,
  `load_root`, `Config::memory`, and the `tokio` dependency go, and nothing is
  kept in memory.
- The defaults merge goes, with its rule that a table whose `provider` or
  `model` key differs from the default is replaced whole.
- A save writes only the keys that changed and keeps unknown keys and the
  file's comments; today a save rewrites a whole table from its type, dropping
  both, and makes a change visible even when writing it failed.
- A file that exists but can't be read fails instead of being replaced.
- Secrets move from one credential per key to one encrypted file whose key is
  the only credential. Keys saved by today's Koharu aren't read, so they're
  entered again; there's no upgrade code.
- The agent's splitting of long values goes, as the file has no size limit.
- Linux keeps the key in the Secret Service instead of the kernel keyring,
  which loses it on reboot; `secrecy`'s `SecretString` replaces the old one,
  which serialized as `"[REDACTED]"`.

## Sources

[codex ConfigToml]: https://github.com/openai/codex/blob/6fe77fc12346591b8037a8704640e40196b19044/codex-rs/config/src/config_toml.rs#L165-L170
[codex read]: https://github.com/openai/codex/blob/6fe77fc12346591b8037a8704640e40196b19044/codex-rs/app-server/src/config_manager_service.rs#L129-L150
[codex strict]: https://github.com/openai/codex/blob/6fe77fc12346591b8037a8704640e40196b19044/codex-rs/config/src/strict_config.rs#L1-L38
[codex edits]: https://github.com/openai/codex/blob/6fe77fc12346591b8037a8704640e40196b19044/codex-rs/core/src/config/edit.rs#L31-L77
[codex apply]: https://github.com/openai/codex/blob/6fe77fc12346591b8037a8704640e40196b19044/codex-rs/core/src/config/edit.rs#L749-L792
[codex defaults]: https://github.com/openai/codex/blob/6fe77fc12346591b8037a8704640e40196b19044/codex-rs/core/src/config/edit.rs#L879-L908
[codex secrets]: https://github.com/openai/codex/blob/6fe77fc12346591b8037a8704640e40196b19044/codex-rs/secrets/src/lib.rs#L90-L157
[codex secrets key]: https://github.com/openai/codex/blob/6fe77fc12346591b8037a8704640e40196b19044/codex-rs/secrets/src/local.rs#L267-L288
[codex secrets save]: https://github.com/openai/codex/blob/6fe77fc12346591b8037a8704640e40196b19044/codex-rs/secrets/src/local.rs#L247-L265
[codex auth storage]: https://github.com/openai/codex/blob/6fe77fc12346591b8037a8704640e40196b19044/codex-rs/config/src/types.rs#L165-L183
[VS Code contributions]: https://code.visualstudio.com/api/references/contribution-points#contributes.configuration
[VS Code workspace]: https://code.visualstudio.com/api/references/vscode-api#workspace
[VS Code secrets]: https://code.visualstudio.com/api/references/vscode-api#SecretStorage
[serde default]: https://serde.rs/field-attrs.html#default
[Cargo unused keys]: https://github.com/rust-lang/cargo/blob/749d85d9fc53250737d2d8ba0d431a00c44982bb/src/workspace/parser/mod.rs#L198
[serde_ignored]: https://docs.rs/serde_ignored/0.1.14/serde_ignored/
[toml_edit]: https://docs.rs/toml_edit/0.25.17/toml_edit/
[io NotFound]: https://doc.rust-lang.org/std/io/enum.ErrorKind.html#variant.NotFound
[atomicwrites]: https://docs.rs/atomicwrites
[File lock]: https://doc.rust-lang.org/std/fs/struct.File.html#method.lock
[age]: https://docs.rs/age/0.12.1/age/
[keyring v1]: https://docs.rs/keyring/4.2.0/keyring/v1/
[secrecy]: https://docs.rs/secrecy/0.10.3/secrecy/
[Windows credential]: https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentiala
