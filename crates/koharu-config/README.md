# koharu-config

Koharu's settings and secrets. Settings live in `~/.koharu/config.toml`, a
table per component, each read as the component's own type with its own
defaults. Secrets, the API keys and sign-in tokens, live in
`~/.koharu/secrets.age`, encrypted with a key kept in the OS credential store.
