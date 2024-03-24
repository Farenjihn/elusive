//! Newc cpio implementation
//!
//! This module implements the cpio newc format
//! that can be used with the Linux kernel to
//! load an initramfs.

use crate::vfs::{Entry, Metadata};

use log::trace;
use std::ffi::CString;
use std::io;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// Magic number for newc cpio files.
const MAGIC: &[u8] = b"070701";
/// Magic bytes for cpio trailer entries.
const TRAILER: &str = "TRAILER!!!";

/// Offset for inode number to avoid reserved inodes (arbitrary).
const INO_OFFSET: u64 = 1337;

/// Represents a cpio archive.
#[derive(PartialEq, Debug)]
pub struct Archive {
    entries: Vec<(PathBuf, Entry)>,
}

impl Archive {
    /// Serialize this entry into cpio newc format.
    pub fn serialize(mut self) -> Result<Vec<u8>, io::Error> {
        self.entries.sort_by(|l, r| l.0.cmp(&r.0));

        let mut newc = NewcSerializer::new();
        for (path, entry) in self.entries {
            newc.serialize_entry(&path, entry)?;
        }

        // add trailer entry at the end of the archive
        newc.serialize_entry(Path::new(TRAILER), Entry::directory())?;
        Ok(newc.into_inner())
    }
}

impl<T> From<T> for Archive
where
    T: IntoIterator<Item = (PathBuf, Entry)>,
{
    fn from(value: T) -> Self {
        let entries = value.into_iter().collect();

        Archive { entries }
    }
}

struct NewcSerializer {
    count: u64,
    buf: Vec<u8>,
}

impl NewcSerializer {
    fn new() -> Self {
        NewcSerializer {
            count: 0,
            buf: Vec::new(),
        }
    }

    fn serialize_entry(&mut self, path: &Path, entry: Entry) -> Result<(), io::Error> {
        if path == Path::new("/") {
            return Ok(());
        }

        trace!("Serializing entry: {:?}", entry);
        let Metadata {
            mode,
            uid,
            gid,
            nlink,
            mtime,
            dev_major,
            dev_minor,
            rdev_major,
            rdev_minor,
        } = entry.metadata;

        // get rid of root / for non-trailer entries
        let path = if path == Path::new(TRAILER) {
            path
        } else {
            path.strip_prefix("/").expect("path is under root")
        };

        // serialize the header for this entry
        let filename = CString::new(path.as_os_str().as_bytes())?.into_bytes_with_nul();
        let filename_len = filename.len();

        let ino = self.count + INO_OFFSET;
        self.count += 1;

        let file_size = match &entry.data {
            Some(data) => data.len(),
            None => 0,
        };

        // magic + 8 * fields + filename + file
        self.buf.reserve(6 + (13 * 8) + filename.len() + file_size);
        self.buf.write_all(MAGIC)?;
        write!(self.buf, "{ino:08x}")?;
        write!(self.buf, "{mode:08x}")?;
        write!(self.buf, "{uid:08x}")?;
        write!(self.buf, "{gid:08x}")?;
        write!(self.buf, "{nlink:08x}")?;
        write!(self.buf, "{mtime:08x}")?;
        write!(self.buf, "{file_size:08x}")?;
        write!(self.buf, "{dev_major:08x}")?;
        write!(self.buf, "{dev_minor:08x}")?;
        write!(self.buf, "{rdev_major:08x}")?;
        write!(self.buf, "{rdev_minor:08x}")?;
        write!(self.buf, "{filename_len:08x}")?;
        write!(self.buf, "{:08x}", 0)?; // CRC, null bytes with our MAGIC
        self.buf.write_all(&filename)?;
        pad_buf(&mut self.buf);

        if let Some(data) = entry.data {
            self.buf.write_all(&data)?;
            pad_buf(&mut self.buf);
        }

        Ok(())
    }

    fn into_inner(self) -> Vec<u8> {
        self.buf
    }
}

// pad the buffer so entries align according to cpio requirements.
fn pad_buf(buf: &mut Vec<u8>) {
    let rem = buf.len() % 4;

    if rem != 0 {
        buf.resize(buf.len() + (4 - rem), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::vfs::Entry;

    #[test]
    fn test_serialize() {
        let mut serializer = NewcSerializer::new();

        let entry = Entry::file(b"data".to_vec());
        serializer
            .serialize_entry(Path::new("/test"), entry)
            .unwrap();

        let buf = serializer.into_inner();
        assert!(!buf.is_empty());
    }
}
