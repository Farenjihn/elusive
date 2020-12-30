use anyhow::{bail, Result};
use goblin::elf::Elf;
use log::error;
use std::collections::HashSet;
use std::ffi::{CStr, CString, OsStr};
use std::fs;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

pub(crate) struct Resolver<'a> {
    paths: &'a [PathBuf],
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(paths: &'a [PathBuf]) -> Self {
        Resolver { paths }
    }

    pub(crate) fn resolve(&self) -> Result<HashSet<PathBuf>> {
        let mut resolved = HashSet::new();

        for path in self.paths {
            let data = fs::read(path)?;
            let elf = match Elf::parse(&data) {
                Ok(elf) => elf,
                Err(_) => {
                    error!("Failed to parse binary: {}", path.display());
                    bail!("only ELF binaries are supported");
                }
            };

            for lib in elf.libraries {
                walk_linkmap(lib, &mut resolved)?;
            }
        }

        Ok(resolved)
    }
}

fn walk_linkmap(lib: &str, resolved: &mut HashSet<PathBuf>) -> Result<()> {
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

        // Walk back to the beginning of the link map
        while !(*linkmap).l_prev.is_null() {
            linkmap = (*linkmap).l_prev as *mut link_map;
        }

        // Skip first entry in linkmap since its name is empty
        // Next entry is also skipped since it is the vDSO
        linkmap = (*linkmap).l_next as *mut link_map;

        // Walk through the link map and add entries
        while !(*linkmap).l_next.is_null() {
            linkmap = (*linkmap).l_next as *mut link_map;
            names.push(CStr::from_ptr((*linkmap).l_name));
        }
    };

    for name in names {
        let path = PathBuf::from(OsStr::from_bytes(name.to_bytes()));
        resolved.insert(path);
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
