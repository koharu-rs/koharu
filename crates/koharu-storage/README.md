# koharu-storage

`koharu-storage` owns Koharu's document: one `.loro` file holding the Loro
snapshot of a project, page images included. Storage owns locking, loading, and
atomic writes. What the document means, what to save, and when belong to
`koharu-scene`.

## Loading

`Document::open` locks the document, maps the file, and imports it with
`LoroDoc::from_snapshot`. Loro copies the whole file into memory, history
included, and keeps that copy while the document is open. It decodes each
container only when it is first read, so a page's images are decoded only when
that page is used. A file that is not a Loro snapshot fails to open.

## Saving

`save` writes a snapshot the scene exported to a synced temporary file and
renames it over the document, so the file is never written in place. Exporting
happens before the call, so the scene can release the document while the file
is written. Loro exports containers that were never read from their loaded
bytes, so exporting does not decode images.

## Locking

`Book.loro` is locked through the hidden `.Book.loro.lock` beside it, so every
save can replace the document without unlocking it. Opening a document that
another Koharu window holds fails with `Error::Locked`. The lock is advisory, so
it keeps out other Koharu windows, not other programs. `Document::remove`
deletes a closed document, then its lock file.
