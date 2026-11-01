# koharu-canvas

Koharu's editor viewport in the browser, on WebGPU and WASM. It draws the
frames the native app prepares onto an HTML canvas and owns only what showing
them takes: GPU resources, the camera, and transient previews. The scene,
commits, undo, and saving stay native, and tools and selection stay in React.
