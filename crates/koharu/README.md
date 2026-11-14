# koharu

Koharu's process.

It starts the app in the order the rest of it depends on, and owns what only
the process can: crash reporting, the logs, the console, and closing on a
panic. The app itself, its window and its commands, is `koharu-desktop`.

## Goal

Every failure is reported, and none leaves the app half running. A crash, a
panic on any thread, and an error the interface hits all reach the same
report, with what led up to it.

## 1. Start

The process starts in this order, each step needing the ones before it:

1. **Crash reporting.** Sentry starts first, with everything on: every error
   and trace, logs, native crash dumps, release health, and the interface's
   events through `koharu-sentry`. Threads copy the main scope when they
   start, so it starts before any of them. Its crash reporter runs this same
   program again to watch the app, so nothing that must run once comes before
   it. A failure while the app starts is the one nothing else would see.
2. **Arguments**, then the panic handler, then the logs.
3. **One async runtime**, which the app's commands and the pipeline's scheduler
   share, so neither waits on the other's scheduler.
4. **The app**, handed the build's Tauri context.

## 2. Diagnostics

- **Logs.** INFO and up go to Sentry as spans, logs, and breadcrumbs, because a
  crash report carries breadcrumbs and not logs, and errors go as issues. The
  same logs go to standard error, filtered by `RUST_LOG`.
- **Panics.** A panic on any thread is reported and sent, then the user is told
  and the app closes, so it never runs on with a thread gone. A thread that
  panicked may have held a lock or left a model half loaded, and an app that
  seems to run but no longer answers loses work silently.
- **The console.** A Windows release build is a windowed program with no
  console of its own. Started from a terminal, it attaches to that terminal's
  console, so its logs still show there.
- **Linux.** CEF's Linux builds read glibc's 32-bit memory statistics, which
  overflow once the models' heap passes 2 GiB, so the program answers that call
  itself with figures that fit.
