//! Initramfs generation
//!
//! This module provides an API to help generating a compressed
//! cpio archive to use as an initramfs.

use crate::config;
use crate::depend;
use crate::kmod::{Kmod, Module, ModuleFormat};
use crate::newc::{Archive, Entry, EntryBuilder};

use anyhow::{bail, Result};
use flate2::read::GzDecoder;
use log::{error, info};
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use xz2::read::XzDecoder;
use zstd::Decoder as ZstdDecoder;

/// Default directories to include in the initramfs
const ROOT_DIRS: [&str; 11] = [
    "/dev", "/etc", "/mnt", "/proc", "/run", "/sys", "/tmp", "/usr", "/usr/bin", "/usr/lib", "/var",
];

/// Default symlinks to create within the initramfs
const ROOT_SYMLINKS: [(&str, &str); 7] = [
    ("/bin", "usr/bin"),
    ("/lib", "usr/lib"),
    ("/lib64", "usr/lib"),
    ("/sbin", "usr/bin"),
    ("/usr/lib64", "lib"),
    ("/usr/sbin", "bin"),
    ("/var/run", "../run"),
];

const DEFAULT_DIR_MODE: u32 = 0o040_000 + 0o755;
const DEFAULT_SYMLINK_MODE: u32 = 0o120_000;

/// Builder for initramfs generation
pub struct InitramfsBuilder {
    /// Entries for the cpio archive
    entries: Vec<Entry>,
    /// Cache of processed paths to avoid duplicates
    cache: HashSet<PathBuf>,
}

impl InitramfsBuilder {
    /// Create a new builder
    pub fn new() -> Result<Self> {
        let mut entries = Vec::new();
        let mut cache = HashSet::new();

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

        let builder = InitramfsBuilder { entries, cache };

        Ok(builder)
    }

    /// Create a new builder from a configuration, optionally providing
    /// a source directory for kernel modules
    pub fn from_config(config: config::Initramfs, module_dir: Option<&Path>) -> Result<Self> {
        let mut builder = InitramfsBuilder::new()?;
        builder.add_init(&config.init)?;

        if let Some(shutdown) = &config.shutdown {
            builder.add_shutdown(shutdown)?;
        }

        if let Some(binaries) = config.bin {
            for binary in binaries {
                builder.add_binary(&binary.path, binary.keep_path)?;
            }
        }

        if let Some(libraries) = config.lib {
            for library in libraries {
                builder.add_library(&library.path)?;
            }
        }

        if let Some(trees) = config.tree {
            for tree in trees {
                builder.add_tree(&tree.copy, &tree.path)?;
            }
        }

        if let Some(symlinks) = config.symlink {
            for symlink in symlinks {
                builder.add_symlink(&symlink.path, &symlink.link)?;
            }
        }

        if let Some(modules) = config.module {
            let mut kmod = if let Some(path) = module_dir {
                if !path.exists() {
                    bail!(
                        "invalid kernel module directory, '{}' does not exist",
                        path.display()
                    );
                }

                Kmod::with_directory(path)
            } else {
                Kmod::new()
            }?;

            for module in modules {
                if let Some(path) = module.path {
                    builder.add_module_from_path(&mut kmod, &path, config.uncompress_modules)?;
                } else if let Some(name) = module.name {
                    builder.add_module_from_name(&mut kmod, &name, config.uncompress_modules)?;
                } else {
                    bail!("invalid kernel module configuration, one of 'name' or 'path' must be specified");
                }
            }
        }

        Ok(builder)
    }

    /// Add the init script from the provided path to the initramfs
    pub fn add_init(&mut self, path: &Path) -> Result<()> {
        if self.cache.contains(path) {
            return Ok(());
        }

        info!("Adding init entrypoint: {}", path.display());
        self.add_entrypoint("init", path)?;

        Ok(())
    }

    /// Add the shutdown script, similar to init
    pub fn add_shutdown(&mut self, path: &Path) -> Result<()> {
        if self.cache.contains(path) {
            return Ok(());
        }

        info!("Adding shutdown entrypoint: {}", path.display());
        self.add_entrypoint("shutdown", path)?;

        Ok(())
    }

    /// Add the binary from the provided path to the initramfs
    /// if keep_path is false, the binary is installed in /usr/bin
    pub fn add_binary(&mut self, path: &Path, keep_path: bool) -> Result<()> {
        if self.cache.contains(path) {
            return Ok(());
        }

        let dest = if keep_path {
            path.parent().unwrap_or_else(|| Path::new("/usr/bin"))
        } else {
            Path::new("/usr/bin")
        };

        info!("Adding binary: {}", path.display());
        self.mkdir_all(dest);
        self.add_elf(path, dest)?;

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

        self.mkdir_all(dest);

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

                    let entry = builder.with_metadata(&metadata).build();

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

                let entry = builder.with_metadata(&metadata).build();

                self.cache.insert(name);
                self.entries.push(entry);
            }
        }

        Ok(())
    }

    /// Add a symlink to the initramfs
    pub fn add_symlink(&mut self, path: &Path, link: &Path) -> Result<()> {
        if self.cache.contains(path) {
            return Ok(());
        }

        if let Some(parent) = path.parent() {
            self.mkdir_all(parent);
        }

        info!("Adding symlink: {} -> {}", path.display(), link.display());
        let entry = EntryBuilder::symlink(path, link)
            .mode(DEFAULT_SYMLINK_MODE)
            .build();

        self.cache.insert(path.to_path_buf());
        self.entries.push(entry);

        Ok(())
    }

    /// Add a named kernel module to the initramfs
    pub fn add_module_from_name(
        &mut self,
        kmod: &mut Kmod,
        name: &str,
        uncompress: bool,
    ) -> Result<()> {
        let module = kmod.module_from_name(name)?;
        let path = module.path()?;

        if self.cache.contains(path) {
            return Ok(());
        }

        info!("Adding module with name: {}", name);
        self.add_module(kmod, module, uncompress)?;

        Ok(())
    }

    /// Add a kernel module to the initramfs from the provided path
    pub fn add_module_from_path(
        &mut self,
        kmod: &mut Kmod,
        path: &Path,
        uncompress: bool,
    ) -> Result<()> {
        let module = kmod.module_from_path(path)?;
        let path = module.path()?;

        if self.cache.contains(path) {
            return Ok(());
        }

        info!("Adding module from path: {}", path.display());
        self.add_module(kmod, module, uncompress)?;

        Ok(())
    }

    /// Return an initramfs from this builder
    pub fn build(self) -> Initramfs {
        Initramfs {
            entries: self.entries,
        }
    }
}

impl InitramfsBuilder {
    fn add_entrypoint(&mut self, name: &str, path: &Path) -> Result<()> {
        if !path.exists() {
            error!("Failed to find {}: {}", name, path.display());
            bail!("{} not found: {}", name, path.display());
        }

        let metadata = fs::metadata(&path)?;
        let data = fs::read(&path)?;

        let entry = EntryBuilder::file(format!("/{}", name), data)
            .with_metadata(&metadata)
            .build();

        self.cache.insert(path.to_path_buf());
        self.entries.push(entry);

        Ok(())
    }

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
            .with_metadata(&metadata)
            .build();

        self.cache.insert(path.to_path_buf());
        self.entries.push(entry);

        Ok(())
    }

    /// Add a module to the initramfs
    fn add_module(&mut self, kmod: &mut Kmod, module: Module, uncompress: bool) -> Result<()> {
        self.mkdir_all(&kmod.dir().join("kernel"));
        let path = module.path()?;

        let metadata = fs::metadata(path)?;
        let data = fs::read(path)?;

        let format = ModuleFormat::from_bytes(&data)?;

        let (filename, data) = if uncompress {
            let filename = format!("{}.ko", module.name()?);
            let data = uncompress_module(&data, &format)?;

            (filename, data)
        } else {
            let filename = format!("{}.{}", module.name()?, format.extension());
            (filename, data)
        };

        let entry = EntryBuilder::file(kmod.dir().join("kernel").join(filename), data)
            .with_metadata(&metadata)
            .build();

        self.cache.insert(path.to_path_buf());
        self.entries.push(entry);

        let info = module.info()?;
        for name in info
            .depends()
            .iter()
            .chain(info.pre_softdeps())
            .chain(info.post_softdeps())
        {
            self.add_module_from_name(kmod, name, uncompress)?;
        }

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
        self.cache.insert(path.to_path_buf());
    }
}

/// Finalized Initramfs
pub struct Initramfs {
    /// Entries for the cpio archive
    entries: Vec<Entry>,
}

impl Initramfs {
    /// Return an archive from this initramfs
    pub fn into_archive(self) -> Archive {
        Archive::new(self.entries)
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
        let mut module = vec![];

        let mut builder = InitramfsBuilder::new()?;
        builder.add_init(Path::new("/sbin/init"))?;

        let ls = PathBuf::from("/bin/ls");
        if ls.exists() {
            builder.add_binary(&ls)?;
            bin.push(config::Binary { path: ls });
        }

        let libc = PathBuf::from("/lib/libc.so.6");
        if libc.exists() {
            builder.add_library(&libc)?;
            lib.push(config::Library { path: libc });
        }

        let hosts = PathBuf::from("/etc/hosts");
        if hosts.exists() {
            builder.add_tree(&[hosts.clone()], Path::new("/etc"))?;
            tree.push(config::Tree {
                path: PathBuf::from("/etc"),
                copy: vec![hosts],
            });
        }

        let udev = PathBuf::from("/lib/udev/rules.d");
        if udev.exists() {
            builder.add_tree(&[udev.clone()], Path::new("/lib/udev/rules.d"))?;
            tree.push(config::Tree {
                path: PathBuf::from("/lib/udev/rules.d"),
                copy: vec![udev],
            });
        }

        let mut kmod = Kmod::new()?;
        let btrfs = kmod.module_from_name("btrfs")?;

        if btrfs.path().is_ok() {
            builder.add_module(&mut kmod, btrfs, false)?;
            module.push(config::Module {
                name: Some("btrfs".to_string()),
                path: None,
            })
        }

        let config = config::Initramfs {
            init: PathBuf::from("/sbin/init"),
            shutdown: None,
            bin: if bin.is_empty() { None } else { Some(bin) },
            lib: if lib.is_empty() { None } else { Some(lib) },
            tree: if tree.is_empty() { None } else { Some(tree) },
            module: if module.is_empty() {
                None
            } else {
                Some(module)
            },
            uncompress_modules: false,
        };

        assert_eq!(
            builder.build().into_archive(),
            InitramfsBuilder::from_config(config, None)?
                .build()
                .into_archive(),
        );

        Ok(())
    }
}

fn uncompress_module(data: &[u8], format: &ModuleFormat) -> Result<Vec<u8>> {
    let mut buf = Vec::new();

    match format {
        ModuleFormat::Elf => buf.extend(data),
        ModuleFormat::Zstd => {
            let mut decoder = ZstdDecoder::new(data)?;
            decoder.read_to_end(&mut buf)?;
        }
        ModuleFormat::Xz => {
            let mut decoder = XzDecoder::new(data);
            decoder.read_to_end(&mut buf)?;
        }
        ModuleFormat::Gzip => {
            let mut decoder = GzDecoder::new(data);
            decoder.read_to_end(&mut buf)?;
        }
    }

    Ok(buf)
}
