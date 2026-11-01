# koharu-llm

Runs Koharu's language models, locally on llama.cpp or at a hosted provider the
user chose, and the in-process agent that edits a project through tools. It's
built on rig: hosted models are rig's providers and the local engine is one
more rig model, so translation, PaddleOCR-VL OCR, and the agent work the same
whichever model answers. It also lists the models the user can pick, with what
each can do.
