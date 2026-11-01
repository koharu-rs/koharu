# koharu-pipeline

Runs the models that turn a scanned page into a translated one in a
`koharu-scene` project: detection, OCR, translation, inpainting, and
typesetting. It reads the project through a host, writes each step's result to
each page as one edit, and decides which models stay loaded and what runs on
the GPU when.
