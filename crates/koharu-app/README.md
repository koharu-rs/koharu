# koharu-app

`koharu-app` owns Koharu's Tauri-managed application state, command API,
project lifecycle, processing jobs, typed channels, and agent host. It uses
`koharu-desktop` to prepare and publish page frames for the browser canvas.

Rust command signatures are the authoritative frontend contract:

```powershell
cargo run -p koharu-app --bin generate
```

## Temporary Linux CEF focus workaround

After upgrading Tauri, check [issue #16251](https://github.com/tauri-apps/tauri/issues/16251)
and restart the development app with the workaround disabled (debug builds only):

```bash
KOHARU_DISABLE_CEF_FOCUS_WORKAROUND=1 bun dev
```

Type in the project-name field with a physical keyboard, click another app
until Koharu loses its highlight, return, and type again. Repeat several times
and after a fresh launch; debugger-injected keys cannot verify native focus.

If typing remains reliable, delete `src/linux_focus.rs` and `src/linux_focus/`,
their registration in `lib.rs` and hook in `app.rs`, and the direct Linux `cef`
and `x11-dl` dependencies (including unused workspace entries). Let Cargo update
the lockfile, remove this section, and repeat the typing check.
