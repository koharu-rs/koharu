//! The X window the compositor presents to on X11: a child of GTK's window
//! that covers it and takes no input. GTK draws its own window with its GL
//! renderer, and NVIDIA's driver lets one graphics API at a time present to a
//! native window, so a Vulkan swapchain on GTK's window fails there
//! (koharu-rs/koharu#1469). Chromium presents to a child window of its own the
//! same way, on its parent's depth and visual.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use wgpu::rwh::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle, WindowHandle,
    XlibWindowHandle,
};
use winit::window::Window;
use x11rb::{
    COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT,
    connection::Connection as _,
    protocol::{
        shape::{ConnectionExt as _, SK, SO},
        xproto::{
            ClipOrdering, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, WindowClass,
        },
    },
    rust_connection::RustConnection,
};

pub struct Child {
    /// GTK's window, whose display the surface is made on.
    parent: Arc<dyn Window>,
    /// The child's own connection, which the server destroys it with.
    connection: RustConnection,
    window: u32,
}

impl Child {
    /// A child of `parent`'s X window, which [`Child::resize`] sizes; `None`
    /// when `parent` isn't one.
    pub fn new(parent: &Arc<dyn Window>) -> Result<Option<Self>> {
        let RawWindowHandle::Xlib(handle) = parent.window_handle()?.as_raw() else {
            return Ok(None);
        };
        let (connection, _) = x11rb::connect(None).context("failed to connect to X")?;
        let window = connection.generate_id()?;
        connection
            .create_window(
                COPY_DEPTH_FROM_PARENT,
                window,
                handle.window as u32,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                COPY_FROM_PARENT,
                &CreateWindowAux::new(),
            )?
            .check()?;
        // An empty input shape leaves the pointer in GTK's window, which
        // takes the pointer crossing into a child for the pointer leaving.
        connection
            .shape_rectangles(
                SO::SET,
                SK::INPUT,
                ClipOrdering::UNSORTED,
                window,
                0,
                0,
                &[],
            )?
            .check()?;
        connection.map_window(window)?.check()?;
        Ok(Some(Self {
            parent: Arc::clone(parent),
            connection,
            window,
        }))
    }

    /// Covers GTK's window, of `size`; it waits for X, so a swapchain made
    /// after it is made for the new size.
    pub fn resize(&self, (width, height): (u32, u32)) -> Result<()> {
        let size = ConfigureWindowAux::new().width(width).height(height);
        self.connection
            .configure_window(self.window, &size)?
            .check()?;
        Ok(())
    }
}

impl HasWindowHandle for Child {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let handle = XlibWindowHandle::new(self.window.into());
        // SAFETY: the window lives as long as the connection `self` holds.
        Ok(unsafe { WindowHandle::borrow_raw(handle.into()) })
    }
}

impl HasDisplayHandle for Child {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.parent.display_handle()
    }
}
