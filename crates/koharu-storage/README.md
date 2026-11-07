# koharu-storage

Koharu's project file: one `.loro` file holding a project's Loro snapshot, page images included.
It locks, loads, and writes the file atomically. What the document means and when to save it
belong to `koharu-scene`.

## API

| Item | What it does |
|---|---|
| `Document::create(path, snapshot)` | Creates a document holding a Loro snapshot, and fails if `path` exists |
| `Document::open(path)` | Opens a document and returns it with its `LoroDoc` |
| `Document::save(&self, snapshot)` | Replaces the document with a snapshot |
| `Document::remove(path)` | Deletes a closed document, then its lock file |
| `Document::path(&self)` | Where it is |

An open `Document` holds the file's lock until it drops.

## Locking

The lock is a hidden `.<name>.lock` beside the document, taken without waiting. A second window
gets `Error::Locked` at once. The lock can't be on the document itself, because a save renames a
new file over it, and the lock would stay with the replaced file.

The lock is advisory, so it only holds off other Koharu windows.

## Reading

`open` maps the file and hands it to Loro, which copies it into memory and decodes each container
when it's first read, so page images stay encoded until a page is used. Mapping saves holding a
second full copy while Loro makes its own, and the map is dropped before `open` returns. Another
program truncating the file during that import would fault.

## Writing

A save goes to a synced temporary file that is renamed over the document, through atomicwrites, so
the document is never written in place.
