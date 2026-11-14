# koharu-config

Koharu's settings and secrets.

Settings are one TOML file meant to be edited by hand, and secrets are one
file encrypted with a key the operating system keeps. Where it follows an
established product, it cites it; the links are under
[Sources](#sources).

## Goal

A setting means one thing wherever it's read, and a bad file is never read as
an empty one. Each component owns its settings and their defaults, so no
central list of them can drift from the component.

## 1. Settings

| Part | Model | Cite |
|---|---|---|
| file | `~/.koharu/config.toml`, a table per component, which a user can edit to set what the app has no screen for yet | Codex keeps its settings in `~/.codex/config.toml` [Codex config] |
| owner | each component owns its table, its types, and its defaults, which are what an empty table reads as | |
| reading | from the file every time, with no cache to go stale; a missing file or table is the defaults | |
| writing | under a lock other processes respect, on what the file holds now, writing only what changed, so comments and other tables stay as they are; no change, no write; the file is replaced at once, never half written | |
| overrides only | a value set back to its default leaves the file, so a user who reset it follows later changes to the default | |
| unknown keys | kept on save, with a warning once per key | |
| bad files | a file that isn't TOML or a value of the wrong type is an error, never the defaults | |

A write reads back what it's about to save and saves only if reading would
return exactly what the edit left, so the file can't hold a value that reads
differently.

## 2. Secrets

| Part | Model | Cite |
|---|---|---|
| file | `~/.koharu/secrets.age`: API keys and sign-in tokens, encrypted with age to an X25519 key | age's X25519 recipients [age] |
| key | the only thing in the operating system's credential store, made with the first secret, so a secret can be any length, though a store may limit an entry's size | Electron's safeStorage encrypts with a key the system keeps: in Keychain Access on macOS, and in a secret store such as KWallet on Linux [Electron safeStorage] |
| no fallback | without a credential store there are no secrets: no environment variable, plain-text file, or passphrase stands in. On Linux the store is a Secret Service, such as GNOME Keyring, KWallet, or KeePassXC | |

| Situation | What happens |
|---|---|
| the key is gone, or was made for another file | the file can never be read again, so it's deleted with a warning, and the next secret starts a new key |
| the store can't answer, as when it's locked | an error, and the file stays |
| the file is damaged | an error, and the file stays |

## Sources

Read in November 2026.

- OpenAI: [Codex config]
- Encryption: [age], [Electron safeStorage]

[Codex config]: https://developers.openai.com/codex/config-reference
[age]: https://age-encryption.org/v1
[Electron safeStorage]: https://www.electronjs.org/docs/latest/api/safe-storage
