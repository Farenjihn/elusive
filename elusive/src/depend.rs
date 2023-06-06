use anyhow::{bail, Result};
use log::error;
use object::elf::FileHeader64;
use object::elf::PT_DYNAMIC;
use object::elf::{DT_NEEDED, DT_STRSZ, DT_STRTAB};
use object::read::elf::{Dyn, FileHeader, ProgramHeader};
use object::read::FileKind;
use object::{Endianness, StringTable};
use std::convert::TryInto;
use std::ffi::{CStr, CString, OsStr};
use std::fs;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DependError {
    #[error("only 64bit elf binaries are supported")]
    Not64BitElf,
    #[error("dlopen failed: {0}")]
    DlopenError(String),
    #[error("dlinfo failed")]
    DlinfoError,
}

pub fn resolve(path: &Path) -> Result<Vec<PathBuf>> {
    let data = fs::read(path)?;
    let data = data.as_slice();

    let kind = FileKind::parse(data)?;

    let needed = if kind == FileKind::Elf64 {
        let elf = FileHeader64::<Endianness>::parse(data)?;
        elf_needed(elf, data)
    } else {
        error!("Failed to parse binary");
        bail!(DependError::Not64BitElf);
    }?;

    Ok(needed)
}

fn elf_needed<T>(elf: &T, data: &[u8]) -> Result<Vec<PathBuf>>
where
    T: FileHeader<Endian = Endianness>,
{
    let endian = elf.endian()?;
    let headers = elf.program_headers(endian, data)?;

    let mut strtab = 0;
    let mut strsz = 0;

    let mut offsets = Vec::new();

    for header in headers {
        if header.p_type(endian) == PT_DYNAMIC {
            if let Some(dynamic) = header.dynamic(endian, data)? {
                for entry in dynamic {
                    let d_tag = entry.d_tag(endian).into();

                    if d_tag == DT_STRTAB.into() {
                        strtab = entry.d_val(endian).into();
                    } else if d_tag == DT_STRSZ.into() {
                        strsz = entry.d_val(endian).into();
                    } else if d_tag == DT_NEEDED.into() {
                        offsets.push(entry.d_val(endian).into());
                    }
                }
            }
        }
    }

    let found = headers
        .iter()
        .filter_map(|header| header.data_range(endian, data, strtab, strsz).ok())
        .flatten()
        .next();

    let mut needed = Vec::new();

    if let Some(data) = found {
        let dynstr = StringTable::new(data, 0, data.len() as u64);

        for offset in offsets {
            let offset = offset.try_into()?;
            let name = dynstr.get(offset).expect("offset exists in string table");

            let lib = OsStr::from_bytes(name);
            let path = find_lib(lib)?;

            needed.push(path);
        }
    }

    Ok(needed)
}

fn find_lib(lib: &OsStr) -> Result<PathBuf> {
    let mut linkmap = MaybeUninit::<*mut link_map>::uninit();

    // most distributions do not include /lib64/systemd or /usr/lib64/systemd
    // in ld.so cache and we're assuming a merged /usr
    //
    // yes this is a horrible hack
    let name = if lib.as_bytes().starts_with(b"libsystemd") {
        CString::new([b"/lib64/systemd/", lib.as_bytes()].concat())?
    } else {
        CString::new(lib.as_bytes())?
    };

    let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY) };

    if handle.is_null() {
        let error = unsafe {
            CStr::from_ptr(libc::dlerror())
                .to_str()
                .expect("error should be valid utf8")
        };

        error!("Failed to open handle to dynamic dependency for {:?}", lib);
        bail!(DependError::DlopenError(error.to_string()));
    }

    let ret = unsafe { libc::dlinfo(handle, libc::RTLD_DI_LINKMAP, linkmap.as_mut_ptr().cast()) };

    if ret < 0 {
        error!("Failed to get path to dynamic dependency for {:?}", lib);
        bail!(DependError::DlinfoError);
    }

    let l_name = unsafe {
        let linkmap = linkmap.assume_init();
        CStr::from_ptr((*linkmap).l_name)
    };

    let path = PathBuf::from(OsStr::from_bytes(l_name.to_bytes()));
    Ok(path)
}

// C struct used in `dlinfo` with `RTLD_DI_LINKMAP`
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
    fn test_failure() {
        let path = PathBuf::from("/dev/null");
        assert!(resolve(&path).is_err());
    }

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
                bail!("resolver did not list libc in dependencies");
            }
        }

        Ok(())
    }
}
