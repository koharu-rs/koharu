use std::path::Path;

use anyhow::Result;

pub(super) fn load(path: impl AsRef<Path>, global: bool) -> Result<&'static libloading::Library> {
    let path = path.as_ref();
    let path = dunce::canonicalize(path)?;
    let library = unsafe { open(&path, global) }?;
    // Native backends retain symbols for the process lifetime. Return the same
    // retained handle to packages that need to inspect their loaded runtime.
    Ok(Box::leak(Box::new(library)))
}

#[cfg(windows)]
unsafe fn open(path: &Path, _global: bool) -> Result<libloading::Library, libloading::Error> {
    use libloading::os::windows::{
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, Library,
    };
    unsafe {
        Library::load_with_flags(
            path.as_os_str(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
        .map(Into::into)
    }
}

#[cfg(not(windows))]
unsafe fn open(path: &Path, global: bool) -> Result<libloading::Library, libloading::Error> {
    use libloading::os::unix::{Library, RTLD_GLOBAL, RTLD_LAZY, RTLD_LOCAL};
    let visibility = if global { RTLD_GLOBAL } else { RTLD_LOCAL };
    unsafe { Library::open(Some(path), RTLD_LAZY | visibility).map(Into::into) }
}
