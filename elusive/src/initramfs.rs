//! Initramfs generation.
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
use std::io::Read;
use std::path::{Path, PathBuf};
use std::{fs, io};
use walkdir::WalkDir;
use xz2::read::XzDecoder;
use zstd::Decoder as ZstdDecoder;

/// Default directories to include in the initramfs.
const ROOT_DIRS: [&str; 11] = [
    "/dev", "/etc", "/mnt", "/proc", "/run", "/sys", "/tmp", "/usr", "/usr/bin", "/usr/lib", "/var",
];

/// Default symlinks to create within the initramfs.
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

/// Builder for initramfs generation.
pub struct InitramfsBuilder {
    /// Entries for the cpio archive.
    entries: Vec<Entry>,
    /// Cache of processed paths to avoid duplicates.
    cache: HashSet<PathBuf>,
}

impl InitramfsBuilder {
    fn add_entrypoint(&mut self, name: &str, path: &Path) -> Result<()> {
        if !path.exists() {
            error!("Failed to find {}: {}", name, path.display());
            bail!(io::Error::new(
                io::ErrorKind::NotFound,
                path.display().to_string()
            ));
        }

        let metadata = fs::metadata(path)?;
        let data = fs::read(path)?;

        let entry = EntryBuilder::file(format!("/{name}"), data)
            .with_metadata(&metadata)
            .build();

        self.cache.insert(path.to_path_buf());
        self.entries.push(entry);

        Ok(())
    }

    fn add_module(
        &mut self,
        kmod: &mut Kmod,
        module: &Module,
        uncompress: bool,
        kernel_release: Option<&str>,
    ) -> Result<()> {
        let kmod_dir = if let Some(kernel_release) = kernel_release {
            Path::new("/lib/modules")
                .join(kernel_release)
                .join("kernel")
        } else {
            kmod.dir().join("kernel")
        };
        self.mkdir_all(&kmod_dir);
        let path = module.path()?;

        if self.cache.contains(path) {
            return Ok(());
        }

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

        let entry = EntryBuilder::file(kmod_dir.join(filename), data)
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
            let module = kmod.module_from_name(name)?;
            self.add_module(kmod, &module, uncompress, kernel_release)?;
        }

        Ok(())
    }

    /// Create directory entries by recursively walking the provided path.
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

    /// Create a new builder.
    pub fn new() -> Result<Self> {
        let mut entries = Vec::new();
        let mut cache = HashSet::new();

        info!("Generating skeleton initramfs");

        for dir in &ROOT_DIRS {
            info!("‣ Adding default directory: {}", dir);

            let entry = EntryBuilder::directory(dir).mode(DEFAULT_DIR_MODE).build();

            cache.insert(dir.into());
            entries.push(entry);
        }

        for (src, dest) in &ROOT_SYMLINKS {
            info!("‣ Adding default symlink: {} -> {}", src, dest);

            let entry = EntryBuilder::symlink(src, Path::new(dest))
                .mode(DEFAULT_SYMLINK_MODE)
                .build();

            cache.insert(src.into());
            entries.push(entry);
        }

        let builder = InitramfsBuilder { entries, cache };

        Ok(builder)
    }

    /// Create a new builder from a configuration.
    pub fn from_config(config: &config::Initramfs, modules: &[config::Module]) -> Result<Self> {
        let mut builder = InitramfsBuilder::new()?;
        builder.add_init(&config.init)?;

        if let Some(shutdown) = &config.shutdown {
            builder.add_shutdown(shutdown)?;
        }

        let settings = &config.settings;
        let mut kmod = match &settings.boot_module_path {
            Some(path) => {
                if !path.exists() {
                    bail!(io::Error::new(
                        io::ErrorKind::NotFound,
                        path.display().to_string(),
                    ));
                }

                Kmod::with_directory(path)
            }
            None => Kmod::new(),
        }?;

        for module in modules {
            info!(
                "Reading configuration module: {}",
                &module.name.as_deref().unwrap_or("<unnamed>")
            );

            for binary in &module.binaries {
                builder.add_elf(&binary.path)?;
            }

            for spec in &module.files {
                builder.add_files(&spec.sources, &spec.destination)?;
            }

            for symlink in &module.symlinks {
                builder.add_symlink(&symlink.source, &symlink.destination)?;
            }

            for module in &module.boot_modules {
                use config::BootModule;

                match module {
                    BootModule::Name(name) => {
                        builder.add_module_from_name(
                            &mut kmod,
                            name,
                            settings.decompress_modules,
                            settings.kernel_release.as_deref(),
                        )?;
                    }
                    BootModule::Path(path) => {
                        builder.add_module_from_path(
                            &mut kmod,
                            path,
                            settings.decompress_modules,
                            settings.kernel_release.as_deref(),
                        )?;
                    }
                }
            }
        }

        Ok(builder)
    }

    /// Add the init script from the provided path to the initramfs.
    pub fn add_init(&mut self, path: &Path) -> Result<()> {
        if self.cache.contains(path) {
            return Ok(());
        }

        info!("‣ Adding init entrypoint: {}", path.display());
        self.add_entrypoint("init", path)?;

        Ok(())
    }

    /// Add the shutdown script, similar to init.
    pub fn add_shutdown(&mut self, path: &Path) -> Result<()> {
        if self.cache.contains(path) {
            return Ok(());
        }

        info!("‣ Adding shutdown entrypoint: {}", path.display());
        self.add_entrypoint("shutdown", path)?;

        Ok(())
    }

    /// Adds an elf binary to the initramfs, also adding its dynamic dependencies.
    pub fn add_elf(&mut self, path: &Path) -> Result<()> {
        if self.cache.contains(path) {
            return Ok(());
        }

        let dest = path
            .parent()
            .expect("parent path exists when keep_path set to true");

        info!("‣ Adding binary: {}", path.display());
        self.mkdir_all(dest);
        if !path.exists() {
            error!("Failed to find binary: {}", path.display());
            bail!(io::Error::new(
                io::ErrorKind::NotFound,
                path.display().to_string()
            ));
        }

        let Some(filename) = path.file_name() else {
            error!("Failed to get filename for binary: {}", path.display());
            bail!(io::Error::new(
                io::ErrorKind::NotFound,
                path.display().to_string()
            ));
        };

        let name = dest.join(filename);
        let metadata = fs::metadata(path)?;
        let data = fs::read(path)?;

        let entry = EntryBuilder::file(name, data)
            .with_metadata(&metadata)
            .build();

        self.cache.insert(path.to_path_buf());
        self.entries.push(entry);

        for dependency in depend::resolve(path)? {
            self.add_elf(&dependency)?;
        }

        Ok(())
    }

    /// Add the filesystem tree from the provided source to the provided destination in the.
    /// initramfs.
    pub fn add_files(&mut self, sources: &[PathBuf], destination: &Path) -> Result<()> {
        info!("‣ Copying files:");
        self.mkdir_all(destination);

        for source in sources {
            info!("    ├─ source = {}", source.display());
            if !source.exists() {
                error!("Failed to find tree: {}", source.display());
                bail!(io::Error::new(
                    io::ErrorKind::NotFound,
                    source.display().to_string()
                ));
            }

            let metadata = fs::metadata(source)?;
            let ty = metadata.file_type();

            if ty.is_dir() {
                let walk = WalkDir::new(source).min_depth(1);

                for entry in walk {
                    let entry = entry?;

                    let path = entry.path();
                    let name = destination.join(
                        path.strip_prefix(source)
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
                        let data = fs::read(path)?;
                        EntryBuilder::file(&name, data)
                    } else if ty.is_symlink() {
                        let data = fs::read_link(path)?;
                        EntryBuilder::symlink(&name, &data)
                    } else {
                        EntryBuilder::special_file(&name)
                    };

                    let entry = builder.with_metadata(&metadata).build();

                    self.cache.insert(name);
                    self.entries.push(entry);
                }
            } else {
                let name =
                    destination.join(source.file_name().expect("path should contain file name"));

                if self.cache.contains(&name) {
                    return Ok(());
                }

                let builder = if ty.is_file() {
                    let data = fs::read(source)?;
                    EntryBuilder::file(&name, data)
                } else if ty.is_symlink() {
                    let data = fs::read_link(source)?;
                    EntryBuilder::symlink(&name, &data)
                } else {
                    EntryBuilder::special_file(&name)
                };

                let entry = builder.with_metadata(&metadata).build();

                self.cache.insert(name);
                self.entries.push(entry);
            }
        }

        info!("    └─ destination = {}", destination.display());

        Ok(())
    }

    /// Add a symlink to the initramfs.
    pub fn add_symlink(&mut self, source: &Path, destination: &Path) -> Result<()> {
        if self.cache.contains(destination) {
            return Ok(());
        }

        if let Some(parent) = destination.parent() {
            self.mkdir_all(parent);
        }

        info!("‣ Adding symlink:");
        info!("    ├─ source = {}", source.display());
        info!("    └─ destination = {}", destination.display());
        let entry = EntryBuilder::symlink(destination, source)
            .mode(DEFAULT_SYMLINK_MODE)
            .build();

        self.cache.insert(destination.to_path_buf());
        self.entries.push(entry);

        Ok(())
    }

    /// Add a named kernel module to the initramfs.
    pub fn add_module_from_name(
        &mut self,
        kmod: &mut Kmod,
        name: &str,
        uncompress: bool,
        kernel_release: Option<&str>,
    ) -> Result<()> {
        let module = kmod.module_from_name(name)?;

        info!("‣ Adding boot module with name: {}", name);
        self.add_module(kmod, &module, uncompress, kernel_release)?;

        Ok(())
    }

    /// Add a kernel module to the initramfs from the provided path.
    pub fn add_module_from_path(
        &mut self,
        kmod: &mut Kmod,
        path: &Path,
        uncompress: bool,
        kernel_release: Option<&str>,
    ) -> Result<()> {
        let module = kmod.module_from_path(path)?;

        info!("‣ Adding boot module from path: {}", path.display());
        self.add_module(kmod, &module, uncompress, kernel_release)?;

        Ok(())
    }

    /// Return an initramfs from this builder.
    pub fn build(self) -> Initramfs {
        Initramfs {
            entries: self.entries,
        }
    }
}

/// Finalized Initramfs.
pub struct Initramfs {
    /// Entries for the cpio archive.
    entries: Vec<Entry>,
}

impl Initramfs {
    /// Return an archive from this initramfs.
    pub fn into_archive(self) -> Archive {
        Archive::new(self.entries)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    #[test]
    fn test_initramfs() -> Result<()> {
        let mut binaries = Vec::new();
        let mut files = Vec::new();
        let mut boot_modules = Vec::new();

        let mut builder = InitramfsBuilder::new()?;
        builder.add_init(Path::new("/sbin/init"))?;

        let ls = PathBuf::from("/bin/ls");
        if ls.exists() {
            builder.add_elf(&ls)?;
            binaries.push(config::Binary { path: ls });
        }

        let libc = PathBuf::from("/lib/libc.so.6");
        if libc.exists() {
            builder.add_elf(&libc)?;
            binaries.push(config::Binary { path: libc });
        }

        let hosts = PathBuf::from("/etc/hosts");
        if hosts.exists() {
            builder.add_files(&[hosts.clone()], Path::new("/etc"))?;
            files.push(config::File {
                destination: PathBuf::from("/etc"),
                sources: vec![hosts],
            });
        }

        let udev = PathBuf::from("/lib/udev/rules.d");
        if udev.exists() {
            builder.add_files(&[udev.clone()], Path::new("/lib/udev/rules.d"))?;
            files.push(config::File {
                destination: PathBuf::from("/lib/udev/rules.d"),
                sources: vec![udev],
            });
        }

        let mut kmod = Kmod::new()?;
        let btrfs = kmod.module_from_name("btrfs")?;

        if btrfs.path().is_ok() {
            builder.add_module(&mut kmod, &btrfs, false, None)?;
            boot_modules.push(config::BootModule::Name("btrfs".to_string()));
        }

        let config = config::Initramfs {
            init: PathBuf::from("/sbin/init"),
            shutdown: None,
            settings: config::Settings::default(),
            module: None,
        };

        let modules = vec![config::Module {
            name: None,
            binaries,
            files,
            boot_modules,
            symlinks: Vec::new(),
        }];

        assert_eq!(
            builder.build().into_archive(),
            InitramfsBuilder::from_config(&config, &modules)?
                .build()
                .into_archive(),
        );

        Ok(())
    }
}
