//! Initramfs generation
//!
//! This module provides an API to help generating a compressed
//! cpio archive to use as an initramfs.

use crate::config;
use crate::depend;
use crate::newc::{Archive, Entry, EntryBuilder};

use anyhow::{bail, Result};
use log::{error, info};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Default directories to include in the initramfs
const ROOT_DIRS: [&str; 11] = [
    "/dev", "/etc", "/mnt", "/proc", "/run", "/sys", "/tmp", "/usr", "/usr/bin", "/usr/lib", "/var",
];

/// Default symlinks to create within the initramfs
const ROOT_SYMLINKS: [(&str, &str); 6] = [
    ("/bin", "usr/bin"),
    ("/lib", "usr/lib"),
    ("/lib64", "usr/lib"),
    ("/sbin", "usr/bin"),
    ("/usr/lib64", "lib"),
    ("/usr/sbin", "bin"),
];

const DEFAULT_DIR_MODE: u32 = 0o040000 + 0o755;
const DEFAULT_SYMLINK_MODE: u32 = 0o120000;

/// Builder pattern for initramfs generation
pub struct Initramfs {
    /// Entries for the cpio archive
    entries: Vec<Entry>,
    /// Cache of processed paths to avoid duplicates
    cache: HashSet<PathBuf>,
}

impl Initramfs {
    /// Create a new initramfs
    pub fn new() -> Result<Self> {
        let mut cache = HashSet::new();
        let mut entries = Vec::new();

        for dir in &ROOT_DIRS {
            info!("Adding default directory: {}", dir);

            let entry = EntryBuilder::directory(dir).mode(DEFAULT_DIR_MODE).build();

            cache.insert(dir.into());
            entries.push(entry);
        }

        for (src, dest) in &ROOT_SYMLINKS {
            info!("Adding default symlink: {} -> {}", src, dest);

            let entry = EntryBuilder::symlink(src, Path::new(dest))
                .mode(DEFAULT_SYMLINK_MODE)
                .build();

            cache.insert(src.into());
            entries.push(entry);
        }

        let initramfs = Initramfs { entries, cache };
        Ok(initramfs)
    }

    /// Create a new initramfs from a configuration
    pub fn from_config(config: config::Initramfs) -> Result<Self> {
        let mut initramfs = Initramfs::new()?;
        initramfs.add_init(&config.init)?;

        if let Some(binaries) = config.bin {
            let paths: Vec<PathBuf> = binaries.into_iter().map(|bin| bin.path).collect();

            for path in paths {
                initramfs.add_binary(&path)?;
            }
        }

        if let Some(libraries) = config.lib {
            let paths: Vec<PathBuf> = libraries.into_iter().map(|lib| lib.path).collect();

            for path in paths {
                initramfs.add_library(&path)?;
            }
        }

        if let Some(trees) = config.tree {
            for tree in trees {
                initramfs.add_tree(&tree.copy, &tree.path)?;
            }
        }

        Ok(initramfs)
    }

    /// Add the init (script or binary) from the provided path to the initramfs
    pub fn add_init(&mut self, path: &Path) -> Result<()> {
        if self.cache.contains(path) {
            return Ok(());
        }

        info!("Adding init script: {}", path.display());

        if !path.exists() {
            error!("Failed to find init: {}", path.display());
            bail!("init not found: {}", path.display());
        }

        let metadata = fs::metadata(&path)?;
        let data = fs::read(&path)?;

        let entry = EntryBuilder::file("/init", data)
            .with_metadata(metadata)
            .build();

        self.cache.insert(path.to_path_buf());
        self.entries.push(entry);

        Ok(())
    }

    /// Add the binary from the provided path to the initramfs
    pub fn add_binary(&mut self, path: &Path) -> Result<()> {
        if self.cache.contains(path) {
            return Ok(());
        }

        info!("Adding binary: {}", path.display());
        self.add_elf(path, Path::new("/usr/bin"))?;

        for dependency in depend::resolve(path)? {
            self.add_library(&dependency)?;
        }

        Ok(())
    }

    /// Add the library from the provided path to the initramfs
    pub fn add_library(&mut self, path: &Path) -> Result<()> {
        if self.cache.contains(path) {
            return Ok(());
        }

        info!("Adding library: {}", path.display());
        self.add_elf(path, Path::new("/usr/lib"))?;

        for dependency in depend::resolve(path)? {
            self.add_library(&dependency)?;
        }

        Ok(())
    }

    /// Add the filesystem tree from the provided source to the provided destination in the
    /// initramfs
    pub fn add_tree(&mut self, copy: &[PathBuf], dest: &Path) -> Result<()> {
        info!("Copying filesystem tree into: {}", dest.display());

        self.mkdir_all(&dest);

        for tree in copy {
            if !tree.exists() {
                error!("Failed to find tree: {}", tree.display());
                bail!("tree not found: {}", tree.display());
            }

            let metadata = fs::metadata(&tree)?;
            let ty = metadata.file_type();

            if ty.is_dir() {
                let walk = WalkDir::new(&tree).min_depth(1);

                for entry in walk {
                    let entry = entry?;

                    let path = entry.path();
                    let name = dest.join(
                        path.strip_prefix(&tree)
                            .expect("entry should be under root path"),
                    );

                    if self.cache.contains(&name) {
                        continue;
                    }

                    let metadata = entry.metadata()?;
                    let ty = metadata.file_type();

                    let builder = if ty.is_dir() {
                        EntryBuilder::directory(&name)
                    } else if ty.is_file() {
                        let data = fs::read(&path)?;
                        EntryBuilder::file(&name, data)
                    } else if ty.is_symlink() {
                        let data = fs::read_link(&path)?;
                        EntryBuilder::symlink(&name, &data)
                    } else {
                        EntryBuilder::special_file(&name)
                    };

                    let entry = builder.with_metadata(metadata).build();

                    self.cache.insert(name);
                    self.entries.push(entry);
                }
            } else {
                let name = dest.join(tree.file_name().expect("path should contain file name"));

                if self.cache.contains(&name) {
                    return Ok(());
                }

                let builder = if ty.is_file() {
                    let data = fs::read(&tree)?;
                    EntryBuilder::file(&name, data)
                } else if ty.is_symlink() {
                    let data = fs::read_link(&tree)?;
                    EntryBuilder::symlink(&name, &data)
                } else {
                    EntryBuilder::special_file(&name)
                };

                let entry = builder.with_metadata(metadata).build();

                self.cache.insert(name);
                self.entries.push(entry);
            }
        }

        Ok(())
    }

    /// Return an archive from this initramfs
    pub fn build(self) -> Archive {
        Archive::new(self.entries)
    }
}

impl Initramfs {
    /// Adds an elf binary to the initramfs, also adding its dynamic dependencies
    fn add_elf(&mut self, path: &Path, dest: &Path) -> Result<()> {
        if !path.exists() {
            error!("Failed to find binary: {}", path.display());
            bail!("binary not found: {}", path.display());
        }

        let filename = match path.file_name() {
            Some(filename) => filename,
            None => {
                error!("Failed to get filename for binary: {}", path.display());
                bail!("filename not found in path: {}", path.display());
            }
        };

        let name = dest.join(filename);
        let metadata = fs::metadata(&path)?;
        let data = fs::read(&path)?;

        let entry = EntryBuilder::file(name, data)
            .with_metadata(metadata)
            .build();

        self.cache.insert(path.to_path_buf());
        self.entries.push(entry);

        Ok(())
    }

    /// Create directory entries by recursively walking the provided path
    fn mkdir_all(&mut self, path: &Path) {
        if self.cache.contains(path) {
            return;
        }

        if path == Path::new("/") {
            return;
        }

        if let Some(parent) = path.parent() {
            self.mkdir_all(parent);
        }

        let entry = EntryBuilder::directory(path).mode(DEFAULT_DIR_MODE).build();
        self.entries.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    #[test]
    fn test_initramfs() -> Result<()> {
        let mut bin = vec![];
        let mut lib = vec![];
        let mut tree = vec![];

        let mut initramfs = Initramfs::new()?;
        initramfs.add_init(Path::new("/sbin/init"))?;

        let ls = PathBuf::from("/bin/ls");
        if ls.exists() {
            initramfs.add_binary(&ls)?;
            bin.push(config::Binary { path: ls });
        }

        let libc = PathBuf::from("/lib/libc.so.6");
        if libc.exists() {
            initramfs.add_library(&libc)?;
            lib.push(config::Library { path: libc });
        }

        let hosts = PathBuf::from("/etc/hosts");
        if hosts.exists() {
            initramfs.add_tree(&[hosts.clone()], Path::new("/etc"))?;
            tree.push(config::Tree {
                path: PathBuf::from("/etc"),
                copy: vec![hosts],
            });
        }

        let udev = PathBuf::from("/lib/udev/rules.d");
        if udev.exists() {
            initramfs.add_tree(&[udev.clone()], Path::new("/lib/udev/rules.d"))?;
            tree.push(config::Tree {
                path: PathBuf::from("/lib/udev/rules.d"),
                copy: vec![udev],
            });
        }

        let config = config::Initramfs {
            init: PathBuf::from("/sbin/init"),
            bin: if bin.is_empty() { None } else { Some(bin) },
            lib: if lib.is_empty() { None } else { Some(lib) },
            tree: if tree.is_empty() { None } else { Some(tree) },
        };

        assert_eq!(initramfs.build(), Initramfs::from_config(config)?.build());
        Ok(())
    }
}
