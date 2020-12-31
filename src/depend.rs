use anyhow::{bail, Result};
use goblin::elf::Elf;
use log::error;
use std::ffi::{CStr, CString, OsStr};
use std::fs;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub fn resolve(path: &Path) -> Result<Vec<PathBuf>> {
    let mut resolved = Vec::new();

    let data = fs::read(path)?;

    let elf = match Elf::parse(&data) {
        Ok(elf) => elf,
        Err(err) => {
            error!("Failed to parse binary: {}", path.display());
            bail!("only ELF binaries are supported: {}", err);
        }
    };

    for lib in elf.libraries {
        walk_linkmap(lib, &mut resolved)?;
    }

    Ok(resolved)
}

fn walk_linkmap(lib: &str, resolved: &mut Vec<PathBuf>) -> Result<()> {
    let name = CString::new(lib)?;
    let mut linkmap = MaybeUninit::<*mut link_map>::uninit();

    let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY) };
    if handle.is_null() {
        let error = unsafe {
            CStr::from_ptr(libc::dlerror())
                .to_str()
                .expect("error should be valid utf8")
        };

        error!("Failed to open handle to dynamic dependency for {}", lib);
        bail!("dlopen failed: {}", error);
    }

    let ret = unsafe {
        libc::dlinfo(
            handle,
            libc::RTLD_DI_LINKMAP,
            linkmap.as_mut_ptr() as *mut libc::c_void,
        )
    };

    if ret < 0 {
        error!("Failed to get path to dynamic dependency for {}", lib);
        bail!("dlinfo failed");
    }

    let mut names = Vec::new();
    unsafe {
        let mut linkmap = linkmap.assume_init();

        // walk back to the beginning of the link map
        while !(*linkmap).l_prev.is_null() {
            linkmap = (*linkmap).l_prev as *mut link_map;
        }

        // skip first entry in linkmap since its name is empty
        // next entry is also skipped since it is the vDSO
        linkmap = (*linkmap).l_next as *mut link_map;

        // walk through the link map and add entries
        while !(*linkmap).l_next.is_null() {
            linkmap = (*linkmap).l_next as *mut link_map;
            names.push(CStr::from_ptr((*linkmap).l_name));
        }
    };

    for name in names {
        let path = PathBuf::from(OsStr::from_bytes(name.to_bytes()));
        resolved.push(path);
    }

    let ret = unsafe { libc::dlclose(handle) };
    if ret < 0 {
        error!("Failed to close handle to dynamic dependency for {}", lib);
        bail!("dlclose failed");
    }

    Ok(())
}

/// C struct used in `dlinfo` with `RTLD_DI_LINKMAP`
#[repr(C)]
struct link_map {
    l_addr: u64,
    l_name: *mut libc::c_char,
    l_ld: *mut libc::c_void,
    l_next: *mut libc::c_void,
    l_prev: *mut libc::c_void,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolver() -> Result<()> {
        let ls = PathBuf::from("/bin/ls");

        if ls.exists() {
            let dependencies = resolve(&ls)?;
            let mut found_libc = false;

            for lib in dependencies {
                if lib
                    .file_name()
                    .expect("library path should have filename")
                    .to_str()
                    .expect("filename should be valid utf8")
                    .starts_with("libc")
                {
                    found_libc = true;
                    break;
                }
            }

            if !found_libc {
                bail!("resolver did not list libc in dependencies")
            }
        }

        Ok(())
    }
}
