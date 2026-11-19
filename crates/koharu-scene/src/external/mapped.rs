use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::OsStringExt as _,
    path::{Component, Components, Path, PathBuf, Prefix},
};

use windows::{
    Win32::{
        Foundation::{ERROR_MORE_DATA, NO_ERROR},
        NetworkManagement::WNet::WNetGetConnectionW,
        Storage::FileSystem::GetLogicalDrives,
    },
    core::{PCWSTR, PWSTR},
};

/// Windows canonicalizes a mapped drive into a UNC path. Resolve stored
/// UNC paths only through mappings already present in the current session.
pub(super) fn resolve(path: &Path) -> Option<PathBuf> {
    let drives = unsafe { GetLogicalDrives() };
    (b'A'..=b'Z').find_map(|drive| {
        if drives & (1 << (drive - b'A')) == 0 {
            return None;
        }
        let root = network_root(drive)?;
        let relative = below(path, Path::new(&root))?;
        let mut resolved = OsString::from(format!("{}:\\", char::from(drive)));
        resolved.push(relative.as_path());
        Some(resolved.into())
    })
}

fn network_root(drive: u8) -> Option<OsString> {
    let local = [u16::from(drive), u16::from(b':'), 0];
    let mut remote = vec![0; 260];
    let mut length = remote.len() as u32;
    // The local name is nul-terminated and both output pointers are live
    // for the call. The length is the writable size in UTF-16 code units.
    let get = |remote: &mut [u16], length: &mut u32| unsafe {
        WNetGetConnectionW(
            PCWSTR(local.as_ptr()),
            Some(PWSTR(remote.as_mut_ptr())),
            length,
        )
    };
    let mut status = get(&mut remote, &mut length);
    if status == ERROR_MORE_DATA {
        remote.resize(length as usize, 0);
        status = get(&mut remote, &mut length);
    }
    if status != NO_ERROR {
        return None;
    }
    let end = remote.iter().position(|&unit| unit == 0)?;
    Some(OsString::from_wide(&remote[..end]))
}

fn below<'a>(path: &'a Path, root: &Path) -> Option<Components<'a>> {
    let (server, share, mut parts) = unc_parts(path)?;
    let (host, volume, prefix) = unc_parts(root)?;
    if !server.eq_ignore_ascii_case(host) || !share.eq_ignore_ascii_case(volume) {
        return None;
    }
    for component in prefix {
        match (component, parts.next()) {
            (Component::Normal(a), Some(Component::Normal(b))) if a.eq_ignore_ascii_case(b) => {}
            _ => return None,
        }
    }
    parts
        .clone()
        .all(|part| matches!(part, Component::Normal(_)))
        .then_some(parts)
}

fn unc_parts(path: &Path) -> Option<(&OsStr, &OsStr, Components<'_>)> {
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return None;
    };
    let (server, share) = match prefix.kind() {
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => (server, share),
        _ => return None,
    };
    (parts.next() == Some(Component::RootDir)).then_some((server, share, parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_canonical_unc_path_must_stay_in_the_mapped_folder() {
        let root = Path::new(r"\\NAS\Volume\Comics");
        let path = Path::new(r"\\?\UNC\nas\volume\comics\series\01.png");
        assert_eq!(
            below(path, root).unwrap().as_path(),
            Path::new(r"series\01.png")
        );
        for rejected in [
            r"\\?\UNC\other\volume\comics\01.png",
            r"\\?\UNC\nas\volume\comics-old\01.png",
            r"\\?\UNC\nas\volume\comics\..\outside\01.png",
        ] {
            assert!(below(Path::new(rejected), root).is_none());
        }
        assert!(below(path, Path::new(r"\\NAS\Volume")).is_some());
    }
}
