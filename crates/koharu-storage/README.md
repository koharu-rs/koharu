# koharu-storage

`koharu-storage` owns Koharu's document: one `.loro` file holding the Loro
snapshot of a project, page images included. Storage owns locking, loading, and
atomic saves. What the document means, and when to save or compact it, belong
to `koharu-scene`.

## Loading

`Document::open` locks the document, maps the file, and imports it with
`LoroDoc::from_snapshot`. Loro copies the encoded snapshot into memory but
decodes each container when it is first read, so a page's images are decoded
only when that page is used. A file that is not a Loro snapshot fails to open.

## Saving

Every save exports the whole document to a synced temporary file and renames it
over the document, so the file is never written in place.

- `save` exports a full snapshot. Containers that were never read are written
  from their loaded bytes, so saving does not decode images. The snapshot keeps
  history, including every image an edit replaced.
- `compact` exports a shallow snapshot that drops that history. Loro decodes
  every container to build it, so it is meant for rare moments such as closing
  a document.

## Locking

`Book.loro` is locked through `Book.loro.lock`, so every save can replace the
document without unlocking it. `Document::remove` deletes a closed document and
its lock file.
