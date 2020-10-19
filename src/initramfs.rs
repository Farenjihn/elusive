//! Initramfs generation
//!
//! This module provides an API to help generating a compressed
//! cpio archive to use as an initramfs.

use crate::config::Initramfs;
use crate::depend::Resolver;
use crate::newc::Archive;
use crate::utils;

use anyhow::Result;
use flate2::write::GzEncoder;
use flate2::Compression;
use log::{error, info};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io, os::unix};
use tempfile::TempDir;

/// Default directories to include in the initramfs
const ROOT_DIRS: [&str; 10] = [
    "dev", "etc", "mnt", "proc", "run", "sys", "tmp", "usr/bin", "usr/lib", "var",
];

/// Default symlinks to create within the initramfs
const ROOT_SYMLINKS: [(&str, &str); 5] = [
    ("bin", "usr/bin"),
    ("lib", "usr/lib"),
    ("lib64", "usr/lib"),
    ("sbin", "usr/bin"),
    ("usr/lib64", "lib"),
];

/// Builder pattern for initramfs generation
pub(crate) struct Builder {
    /// Map of entries to avoid duplicates
    map: HashMap<PathBuf, PathBuf>,
}

impl Builder {
    /// Create a new builder
    pub(crate) fn new() -> Result<Self> {
        let builder = Builder {
            map: HashMap::new(),
        };

        Ok(builder)
    }

    /// Create a new builder from a configuration and optional kernel version
    pub(crate) fn from_config(config: Initramfs, kernel_version: Option<String>) -> Result<Self> {
        let mut builder = Builder::new()?;
        builder.add_init(config.init)?;

        if let Some(binaries) = config.bin {
            let paths: Vec<PathBuf> = binaries.into_iter().map(|bin| bin.path).collect();

            let resolver = Resolver::new(&paths);
            let dependencies = resolver.resolve()?;

            for path in paths {
                builder.add_binary(path)?;
            }

            for dependency in dependencies {
                builder.add_library(dependency)?;
            }
        }

        if let Some(libraries) = config.lib {
            let paths: Vec<PathBuf> = libraries.into_iter().map(|lib| lib.path).collect();

            let resolver = Resolver::new(&paths);
            let dependencies = resolver.resolve()?;

            for path in paths {
                builder.add_library(path)?;
            }

            for dependency in dependencies {
                builder.add_library(dependency)?;
            }
        }

        if let Some(trees) = config.tree {
            for tree in trees {
                builder.add_tree(tree.source, tree.dest)?;
            }
        }

        if let Some(modules) = config.module {
            for module in modules {
                let path = modinfo(module.name, &kernel_version)?;
                builder.add_module(path)?;
            }
        }

        Ok(builder)
    }

    /// Add the init (script or binary) from the provided path to the initramfs
    pub(crate) fn add_init(&mut self, path: PathBuf) -> Result<()> {
        info!("Adding init script: {}", path.display());
        self.map.insert(path, PathBuf::from("/init"));

        Ok(())
    }

    /// Add the kernel module from the provided path to the initramfs
    pub(crate) fn add_module(&mut self, path: PathBuf) -> Result<()> {
        if !self.map.contains_key(&path) {
            info!("Adding kernel module: {}", path.display());
            self.map.insert(path.clone(), path);
        }

        Ok(())
    }

    /// Add the binary from the provided path to the initramfs
    pub(crate) fn add_binary(&mut self, path: PathBuf) -> Result<()> {
        if !self.map.contains_key(&path) {
            info!("Adding binary: {}", path.display());
            return self.add_elf(path, PathBuf::from("/usr/bin"));
        }

        Ok(())
    }

    /// Add the library from the provided path to the initramfs
    pub(crate) fn add_library(&mut self, path: PathBuf) -> Result<()> {
        if !self.map.contains_key(&path) {
            info!("Adding library: {}", path.display());
            return self.add_elf(path, PathBuf::from("/usr/lib"));
        }

        Ok(())
    }

    /// Add the filesystem tree from the provided source to the provided destination in the
    /// initramfs
    pub(crate) fn add_tree(&mut self, source: PathBuf, dest: PathBuf) -> Result<()> {
        info!("Copying filesystem tree from: {}", source.display());
        self.map.insert(source, dest);

        Ok(())
    }

    /// Build the initramfs by writing all entries to a temporary directory
    /// and then walking it to create the compressed cpio archive
    pub(crate) fn build<P>(self, output: P, ucode: Option<P>) -> Result<()>
    where
        P: AsRef<Path>,
    {
        let output = output.as_ref();
        info!("Writing initramfs to: {}", output.display());

        let tmp = TempDir::new()?;
        let tmp_path = tmp.path();

        for dir in &ROOT_DIRS {
            fs::create_dir_all(tmp_path.join(dir))?;
        }

        for link in &ROOT_SYMLINKS {
            unix::fs::symlink(link.1, tmp_path.join(link.0))?;
        }

        for (source, to) in self.map {
            let dest = tmp_path.join(to.strip_prefix("/").expect("path should have a leading /"));
            utils::copy_files(&source, dest)?;
        }

        depmod(tmp_path)?;
        let mut output_file = utils::maybe_stdout(&output)?;

        if let Some(ucode) = ucode {
            let ucode = ucode.as_ref();
            info!("Adding microcode bundle from: {}", ucode.display());

            let mut file = utils::maybe_stdin(&ucode)?;
            io::copy(&mut file, &mut output_file)?;
        }

        let mut encoder = GzEncoder::new(output_file, Compression::default());
        Archive::from_root(tmp_path, &mut encoder)?;

        Ok(())
    }
}

impl Builder {
    /// Adds an elf binary to the initramfs, also adding its dynamic dependencies
    fn add_elf(&mut self, source: PathBuf, dest: PathBuf) -> Result<()> {
        if !source.exists() {
            error!("Failed to find binary: {}", source.display());
            anyhow::bail!("binary not found: {}", source.display());
        }

        let filename = match source.file_name() {
            Some(filename) => filename,
            None => {
                error!("Failed to get filename for binary: {}", source.display());
                anyhow::bail!("filename not found in path: {}", source.display());
            }
        };

        let dest = dest.join(filename);
        self.map.insert(source, dest);

        Ok(())
    }
}

/// Run `modinfo` to get path to module from its name
fn modinfo<T>(name: T, kernel_version: &Option<String>) -> Result<PathBuf>
where
    T: AsRef<str>,
{
    let mut command = Command::new("modinfo");

    if let Some(version) = kernel_version {
        command.args(&["-k", &version]);
    }

    let output = command.args(&["-n", name.as_ref()]).output()?;
    let path = std::str::from_utf8(&output.stdout)
        .expect("modinfo output should be valid utf8")
        .trim_end();

    Ok(path.into())
}

/// Run `depmod` to create `modules.dep` and map files
fn depmod(path: &Path) -> Result<()> {
    Command::new("depmod")
        .args(&[
            "-b",
            path.to_str().expect("tmpdir path should be valid utf8"),
        ])
        .output()?;

    Ok(())
}
