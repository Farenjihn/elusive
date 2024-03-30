//! Simple / naive implementation of a virtual filesystem.
//!
//! This VFS is used to back initramfs and microcode archive generation to avoid
//! copying files on disk or in tmpfs.

use std::collections::btree_map::IntoIter;
use std::collections::BTreeMap;
use std::io;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const DIRECTORY_MODE: u32 = 0o040_755;
const FILE_MODE: u32 = 0o100_644;
const SYMLINK_MODE: u32 = 0o120_000;

/// Error returned by VFS.
#[derive(thiserror::Error, Debug)]
pub enum VfsError {
    #[error("no such file or directory: {0}")]
    NoSuchFileOrDirectory(PathBuf),
    #[error("not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("file already exists: {0}")]
    FileExists(PathBuf),
}

/// Representation for VFS entry metadata.
#[derive(Clone, PartialEq, Default, Debug)]
pub struct Metadata {
    /// Mode of the entry.
    pub mode: u32,
    /// User id of the entry.
    pub uid: u64,
    /// Group id of the entry.
    pub gid: u64,
    /// Number of links to the entry.
    pub nlink: u64,
    /// Modification time of the entry.
    pub mtime: u64,
    /// Device major number of the entry.
    pub dev_major: u64,
    /// Device minor number of the entry.
    pub dev_minor: u64,
    /// Rdev major number of the entry.
    pub rdev_major: u64,
    /// Rdev minor number of the entry.
    pub rdev_minor: u64,
}

/// A VFS entry.
#[derive(Clone, PartialEq, Default, Debug)]
pub struct Entry {
    /// Metadata for the entry.
    pub metadata: Metadata,
    /// Data if entry is a regular file or symlink.
    pub data: Option<Vec<u8>>,
}

impl Entry {
    /// Create an entry representing a directory.
    pub fn directory() -> Self {
        Entry {
            metadata: Metadata {
                mode: DIRECTORY_MODE,
                ..Default::default()
            },
            data: None,
        }
    }

    /// Create an entry representing a regular file.
    pub fn file(data: Vec<u8>) -> Self {
        Entry {
            metadata: Metadata {
                mode: FILE_MODE,
                ..Default::default()
            },
            data: Some(data),
        }
    }

    /// Create an entry representing a symlink.
    pub fn symlink<P>(target: P) -> Self
    where
        P: AsRef<Path>,
    {
        let data = target.as_ref().as_os_str().as_bytes().to_vec();

        Entry {
            metadata: Metadata {
                mode: SYMLINK_MODE,
                ..Default::default()
            },
            data: Some(data),
        }
    }

    /// Check if the entry is a directory.
    pub fn is_dir(&self) -> bool {
        self.metadata.mode == DIRECTORY_MODE
    }

    /// Check if the entry is a normal file.
    pub fn is_file(&self) -> bool {
        self.metadata.mode == FILE_MODE
    }

    /// Check if the entry is a symlink.
    pub fn is_symlink(&self) -> bool {
        self.metadata.mode == SYMLINK_MODE
    }
}

impl TryFrom<std::fs::File> for Entry {
    type Error = io::Error;

    fn try_from(mut file: std::fs::File) -> Result<Self, Self::Error> {
        let metadata = file.metadata()?;

        let mut entry = Entry {
            metadata: Metadata {
                mode: metadata.mode(),
                mtime: metadata
                    .mtime()
                    .try_into()
                    .expect("timetstamp does not fit in a u64"),
                rdev_major: major(metadata.rdev()),
                rdev_minor: minor(metadata.rdev()),
                ..Default::default()
            },
            data: None,
        };

        if !metadata.is_dir() {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;

            entry.data = Some(buf);
        }

        Ok(entry)
    }
}

/// Virtual filesystem.
pub struct Vfs {
    inner: BTreeMap<PathBuf, Entry>,
}

impl Vfs {
    /// Create a new VFS with a single root (/) node.
    pub fn new() -> Self {
        let mut map = BTreeMap::new();
        map.insert(PathBuf::from("/"), Entry::directory());

        Vfs { inner: map }
    }

    /// Check the VFS has an entry at the given path.
    pub fn contains<P>(&self, path: P) -> bool
    where
        P: AsRef<Path>,
    {
        self.inner.contains_key(path.as_ref())
    }

    /// Check the VFS contains a directory at given path.
    pub fn contains_dir<P>(&self, path: P) -> bool
    where
        P: AsRef<Path>,
    {
        if let Some(entry) = self.inner.get(path.as_ref()) {
            return entry.is_dir();
        }

        false
    }

    /// Check the VFS contains a file at given path.
    pub fn contains_file<P>(&self, path: P) -> bool
    where
        P: AsRef<Path>,
    {
        if let Some(entry) = self.inner.get(path.as_ref()) {
            return entry.is_file();
        }

        false
    }

    /// Create a directory entry in the VFS.
    pub fn create_dir<P>(&mut self, path: P) -> Result<(), VfsError>
    where
        P: AsRef<Path>,
    {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !self.contains(parent) {
                return Err(VfsError::NoSuchFileOrDirectory(parent.into()));
            }

            // should check if symlink and target
            if self.contains_file(parent) {
                return Err(VfsError::NotADirectory(parent.into()));
            }
        }

        if let Some(entry) = self.inner.get(path) {
            // should check symlink target, for now be lazy
            if entry.is_dir() || entry.is_symlink() {
                return Ok(());
            }

            if entry.is_file() {
                return Err(VfsError::FileExists(path.into()));
            }
        }

        self.inner.insert(path.into(), Entry::directory());
        Ok(())
    }

    /// Recursively create directories in the VFS.
    pub fn create_dir_all<P>(&mut self, path: P) -> Result<(), VfsError>
    where
        P: AsRef<Path>,
    {
        let path = path.as_ref();
        if self.contains_dir(path) {
            return Ok(());
        }

        let ancestors: Vec<&Path> = path.ancestors().collect();
        for dir in ancestors.iter().rev() {
            self.create_dir(dir)?;
        }

        Ok(())
    }

    /// Create an entry in the VFS.
    pub fn create_entry<P>(&mut self, path: P, entry: Entry) -> Result<(), VfsError>
    where
        P: AsRef<Path>,
    {
        let path = path.as_ref();

        if self.contains_file(path) {
            return Err(VfsError::FileExists(path.into()));
        }

        self.inner.insert(path.into(), entry);
        Ok(())
    }
}

impl Default for Vfs {
    fn default() -> Self {
        Self::new()
    }
}

impl IntoIterator for Vfs {
    type Item = (PathBuf, Entry);
    type IntoIter = IntoIter<PathBuf, Entry>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

// shamelessly taken from the `nix` crate !
const fn major(dev: u64) -> u64 {
    ((dev >> 32) & 0xffff_f000) | ((dev >> 8) & 0x0000_0fff)
}

// shamelessly taken from the `nix` crate, thanks !
const fn minor(dev: u64) -> u64 {
    ((dev >> 12) & 0xffff_ff00) | ((dev) & 0x0000_00ff)
}
