# koharu-scene

Koharu's scene graph on Loro. A project is pages, each a tree of groups,
images, rasters, shapes, text, and masks, modeled on what established editors
such as Photoshop and Clip Studio Paint already have. A layer records what a
model wrote, so rerunning a step never undoes what the user changed, and tools
keep their own data in namespaced attributes the renderer never reads.
