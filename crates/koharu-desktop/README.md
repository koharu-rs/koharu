# koharu-desktop

The app: one window whose React interface renders off-screen in CEF and is composited with wgpu over the native canvas, on the window's own surface and from the renderer's GPU, with the window's pointer, keys, and input method forwarded to CEF, and the commands the interface calls against the open project.
