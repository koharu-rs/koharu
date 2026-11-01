# koharu-desktop

Prepares pages natively for the browser canvas and for export. It keeps the
current frame from `koharu-renderer`, publishes new frames to the frontend,
serves each frame's resources, validates transform commits, and owns the
rasterizer that export and previews share. Page requests are latest-wins, so an
outdated request never replaces the frame on screen.
