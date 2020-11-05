//! Newc cpio implementation
//!
//! This module implements the cpio newc format
//! that can be used with the Linux kernel to
//! load an initramfs.

use anyhow::Result;
use std::ffi::CString;
use std::fs;
use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Magic number for newc cpio files
const MAGIC: &[u8] = b"070701";
/// Magic bytes for cpio trailer entries
const TRAILER: &str = "TRAILER!!!";

/// Represents a cpio archive
pub(crate) struct Archive;

impl Archive {
    /// Create an archive from the provided root directory
    ///
    /// This will walk the archive, create all corresponding entries and write them
    /// to a compressed cpio archive.
    pub(crate) fn from_root<P, O>(root_dir: P, out: &mut O) -> Result<()>
    where
        P: AsRef<Path>,
        O: Write,
    {
        let root_dir = root_dir.as_ref();
        let walk = WalkDir::new(&root_dir).into_iter().skip(1).enumerate();

        let mut buf = Vec::new();
        for (index, dir_entry) in walk {
            let dir_entry = dir_entry?;
            let name = dir_entry.path().strip_prefix(&root_dir)?.to_string_lossy();

            let metadata = dir_entry.metadata()?;
            let ty = metadata.file_type();

            let builder = if ty.is_dir() {
                EntryBuilder::directory(&name)
            } else if ty.is_file() {
                let file = File::open(dir_entry.path())?;
                EntryBuilder::file(&name, file)
            } else if ty.is_symlink() {
                let path = fs::read_link(dir_entry.path())?;
                EntryBuilder::symlink(&name, path)
            } else {
                unreachable!();
            };

            let entry = builder.with_metadata(metadata).ino(index as u64).build();
            entry.write_to_buf(&mut buf)?;
        }

        let trailer = EntryBuilder::trailer().ino(0).build();
        trailer.write_to_buf(&mut buf)?;

        out.write_all(&buf)?;

        Ok(())
    }
}

/// Type of a cpio entry
pub(crate) enum EntryType {
    /// Entry is a directory
    Directory,
    /// Entry is a file
    File(File),
    /// Entry is a symlink to a file
    Symlink(PathBuf),
    /// Entry is a trailer delimiter
    Trailer,
}

/// Header for a cpio newc entry
#[derive(Default)]
pub(crate) struct EntryHeader {
    /// Name of the entry (path)
    name: String,
    /// Inode of the entry
    ino: u64,
    /// Mode of the entry
    mode: u32,
    /// Number of links to the entry
    nlink: u64,
    /// Modification time of the entry
    mtime: u64,
    /// Device major number of the entry
    dev_major: u64,
    /// Device minor number of the entry
    dev_minor: u64,
    /// Rdev major number of the entry
    rdev_major: u64,
    /// Rdev minor number of the entry
    rdev_minor: u64,
}

impl EntryHeader {
    /// Create a header with the provided name
    pub(crate) fn with_name<T>(name: T) -> Self
    where
        T: AsRef<str>,
    {
        EntryHeader {
            name: name.as_ref().to_owned(),
            ..EntryHeader::default()
        }
    }
}

/// Cpio newc entry
pub(crate) struct Entry {
    /// Type of the entry
    ty: EntryType,
    /// Newc header for the entry
    header: EntryHeader,
}

impl Entry {
    /// Serialize the header for this entry inside of the passed buffer
    fn write_header(&mut self, file_size: usize, buf: &mut Vec<u8>) -> Result<()> {
        let filename = CString::new(self.header.name.clone())?.into_bytes_with_nul();

        buf.reserve(6 + (13 * 8) + filename.len() + file_size);
        buf.extend(MAGIC);
        write!(buf, "{:08x}", self.header.ino)?;
        write!(buf, "{:08x}", self.header.mode)?;
        // uid is always 0 (root)
        write!(buf, "{:08x}", 0)?;
        // gid is always 0 (root)
        write!(buf, "{:08x}", 0)?;
        write!(buf, "{:08x}", self.header.nlink)?;
        write!(buf, "{:08x}", self.header.mtime)?;
        write!(buf, "{:08x}", file_size)?;
        write!(buf, "{:08x}", self.header.dev_major)?;
        write!(buf, "{:08x}", self.header.dev_minor)?;
        write!(buf, "{:08x}", self.header.rdev_major)?;
        write!(buf, "{:08x}", self.header.rdev_minor)?;
        write!(buf, "{:08x}", filename.len())?;
        write!(buf, "{:08x}", 0)?;
        buf.extend(filename);

        Ok(())
    }
}

impl Entry {
    /// Serialize the entry to the passed buffer
    pub(crate) fn write_to_buf(mut self, mut buf: &mut Vec<u8>) -> Result<()> {
        let file_size = match self.ty {
            EntryType::File(ref mut file) => {
                let file_size = file.seek(SeekFrom::End(0))?;
                file.seek(SeekFrom::Start(0))?;

                file_size as usize
            }
            EntryType::Symlink(ref path) => {
                let path_str = path.to_string_lossy();
                path_str.len()
            }
            _ => 0,
        };

        self.write_header(file_size as usize, &mut buf)?;
        pad_buf(&mut buf);

        match self.ty {
            EntryType::File(ref mut file) => {
                file.read_to_end(&mut buf)?;
            }
            EntryType::Symlink(ref path) => {
                buf.extend(path.to_string_lossy().bytes());
            }
            _ => (),
        }

        pad_buf(&mut buf);
        Ok(())
    }
}

/// Builder pattern for a cpio entry
pub(crate) struct EntryBuilder {
    /// Entry being built
    entry: Entry,
}

impl EntryBuilder {
    /// Create an entry with the directory type
    pub(crate) fn directory<T>(name: T) -> Self
    where
        T: AsRef<str>,
    {
        EntryBuilder {
            entry: Entry {
                ty: EntryType::Directory,
                header: EntryHeader::with_name(name),
            },
        }
    }

    /// Create an entry with the file type
    pub(crate) fn file<T>(name: T, file: File) -> Self
    where
        T: AsRef<str>,
    {
        EntryBuilder {
            entry: Entry {
                ty: EntryType::File(file),
                header: EntryHeader::with_name(name),
            },
        }
    }

    /// Create an entry with the symlink type
    pub(crate) fn symlink<T>(name: T, path: PathBuf) -> Self
    where
        T: AsRef<str>,
    {
        EntryBuilder {
            entry: Entry {
                ty: EntryType::Symlink(path),
                header: EntryHeader::with_name(name),
            },
        }
    }

    /// Create an entry with the trailer type
    pub(crate) fn trailer() -> Self {
        EntryBuilder {
            entry: Entry {
                ty: EntryType::Trailer,
                header: EntryHeader::with_name(TRAILER),
            },
        }
    }

    /// Add the provided metadata to the entry
    pub(crate) fn with_metadata(self, metadata: Metadata) -> Self {
        self.mode(metadata.mode()).mtime(metadata.mtime() as u64)
    }

    /// Set the inode for the entry
    pub(crate) fn ino(mut self, ino: u64) -> Self {
        self.entry.header.ino = ino;
        self
    }

    /// Set the mode for the entry
    pub(crate) fn mode(mut self, mode: u32) -> Self {
        self.entry.header.mode = mode;
        self
    }

    /// Set the modification time for the entry
    pub(crate) fn mtime(mut self, mtime: u64) -> Self {
        self.entry.header.mtime = mtime;
        self
    }

    /// Build the entry
    pub(crate) fn build(self) -> Entry {
        self.entry
    }
}

/// Pad the buffer so entries align according to cpio requirements
pub fn pad_buf(buf: &mut Vec<u8>) {
    let rem = buf.len() % 4;

    if rem != 0 {
        buf.resize(buf.len() + (4 - rem), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_builder() -> Result<()> {
        let temp = NamedTempFile::new()?;
        let temp = temp.into_file();
        let meta = temp.metadata()?;

        let entry = EntryBuilder::file("testfile", temp).ino(0).with_metadata(meta).build();

        let mut buf = Vec::new();
        entry.write_to_buf(&mut buf)?;

        Ok(())
    }
}
