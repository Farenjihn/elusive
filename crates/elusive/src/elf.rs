//! ELF file discovery and management.
//!
//! This module is useful to get the dependencies for a given elf file as well
//! as finding out whether it exists by searching for it in the filesystem.

use crate::search::search_paths;

use log::error;
use object::elf::FileHeader64;
use object::elf::PT_DYNAMIC;
use object::elf::{DT_NEEDED, DT_STRSZ, DT_STRTAB};
use object::read::elf::{Dyn, FileHeader, ProgramHeader};
use object::read::FileKind;
use object::{Endianness, StringTable};
use std::convert::TryInto;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::{fs, io};

const BINARY_SEARCH_PATHS: &[&str] = &[
    "/usr/bin/",
    "/usr/sbin/",
    "/usr/local/bin/",
    "/usr/local/sbin/",
    "/usr/lib/systemd/",
    "/usr/lib/systemd/system-generators/",
    "/bin/",
    "/sbin/",
];

const LIBRARY_SEARCH_PATHS: &[&str] = &[
    "/usr/lib/",
    "/usr/lib64/",
    "/usr/lib/systemd/",
    "/lib/",
    "/lib64",
];

/// Custom error type for elf file processing.
#[derive(thiserror::Error, Debug)]
pub enum ElfError {
    #[error("error reading elf: {0}")]
    InputOutput(io::Error),
    #[error("error parsing elf: {0}")]
    Parsing(object::Error),
    #[error("only 64 bit elf binaries are supported")]
    Not64BitElf,
    #[error("could not find binary: {0:?}")]
    BinaryNotFound(OsString),
    #[error("could not find library: {0:?}")]
    LibraryNotFound(OsString),
}

impl From<io::Error> for ElfError {
    fn from(err: io::Error) -> Self {
        Self::InputOutput(err)
    }
}

impl From<object::Error> for ElfError {
    fn from(err: object::Error) -> Self {
        Self::Parsing(err)
    }
}

/// Utility type for ELF files.
pub struct Elf;

impl Elf {
    /// Get a list of dynamic libraries linked by the ELF file available at the given path.
    pub fn linked_libraries(path: &Path) -> Result<Vec<PathBuf>, ElfError> {
        let data = fs::read(path)?;
        let data = data.as_slice();

        let kind = FileKind::parse(data)?;
        if kind != FileKind::Elf64 {
            error!("Failed to parse binary");
            return Err(ElfError::Not64BitElf);
        }

        let elf = FileHeader64::<Endianness>::parse(data)?;
        let endian = elf.endian()?;
        let headers = elf.program_headers(endian, data)?;

        let mut strtab = 0;
        let mut strsz = 0;

        let mut offsets: Vec<u64> = Vec::new();

        for header in headers {
            if header.p_type(endian) == PT_DYNAMIC {
                if let Some(dynamic) = header.dynamic(endian, data)? {
                    for entry in dynamic {
                        let d_tag = entry.d_tag(endian);

                        if d_tag == DT_STRTAB as u64 {
                            strtab = entry.d_val(endian);
                        } else if d_tag == DT_STRSZ as u64 {
                            strsz = entry.d_val(endian);
                        } else if d_tag == DT_NEEDED as u64 {
                            offsets.push(entry.d_val(endian));
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
                let offset = offset.try_into().expect("offset fits in 32 bits");
                let name = dynstr.get(offset).expect("offset exists in string table");

                let lib = OsStr::from_bytes(name);
                let path = Self::find_library(lib)?;

                needed.push(path);
            }
        }

        Ok(needed)
    }

    /// Find an ELF binary with the given name and return its path if it exists.
    pub fn find_binary<P>(name: P) -> Result<PathBuf, ElfError>
    where
        P: AsRef<Path>,
    {
        search_paths(&name, BINARY_SEARCH_PATHS)
            .ok_or_else(|| ElfError::BinaryNotFound(name.as_ref().into()))
    }

    /// Find an ELF library with the given name and return its path if it exists.
    pub fn find_library<P>(name: P) -> Result<PathBuf, ElfError>
    where
        P: AsRef<Path>,
    {
        search_paths(&name, LIBRARY_SEARCH_PATHS)
            .ok_or_else(|| ElfError::LibraryNotFound(name.as_ref().into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_failure() {
        let path = PathBuf::from("/dev/null");
        assert!(Elf::linked_libraries(&path).is_err());
    }

    #[test]
    fn test_resolver() {
        let ls = PathBuf::from("/bin/ls");

        if ls.exists() {
            let libs = Elf::linked_libraries(&ls).unwrap();
            let mut found_libc = false;

            for lib in libs {
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
                panic!("resolver did not list libc in dependencies");
            }
        }
    }
}
