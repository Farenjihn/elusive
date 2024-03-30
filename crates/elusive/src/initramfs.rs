//! Initramfs generation.
//!
//! This module provides an API to help generating a compressed
//! cpio archive to use as an initramfs.

use crate::config;
use crate::elf::{Elf, ElfError};
use crate::kmod::{Kmod, KmodError, Module, ModuleFormat};
use crate::newc::Archive;
use crate::systemd::{Unit, UnitError};
use crate::vfs::{Entry, Vfs, VfsError};

use flate2::read::GzDecoder;
use log::{debug, error};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::{fs, io};
use walkdir::WalkDir;
use xz2::read::XzDecoder;
use zstd::Decoder as ZstdDecoder;

/// Default directories to include in the initramfs.
const ROOT_DIRS: &[&str] = &[
    "/dev", "/etc", "/proc", "/root", "/run", "/sys", "/tmp", "/usr", "/var",
];

/// Default symlinks to create within the initramfs.
const ROOT_SYMLINKS: &[(&str, &str)] = &[
    ("/bin", "usr/bin"),
    ("/lib", "usr/lib"),
    ("/lib64", "usr/lib"),
    ("/sbin", "usr/bin"),
    ("/usr/lib64", "lib"),
    ("/usr/sbin", "bin"),
    ("/var/run", "../run"),
];

/// Custom error type for initramfs generation.
#[derive(thiserror::Error, Debug)]
pub enum InitramfsError {
    #[error("i/o error: {0}")]
    InputOutput(io::Error),
    #[error("failed to walk directory: {0}")]
    Walk(walkdir::Error),
    #[error("vfs error: {0}")]
    Vfs(VfsError),
    #[error("kernel module error: {0}")]
    Kmod(KmodError),
    #[error("systemd unit error: {0}")]
    System(UnitError),
    #[error("elf error: {0}")]
    Elf(ElfError),
}

impl From<io::Error> for InitramfsError {
    fn from(err: io::Error) -> Self {
        Self::InputOutput(err)
    }
}

impl From<walkdir::Error> for InitramfsError {
    fn from(err: walkdir::Error) -> Self {
        Self::Walk(err)
    }
}

impl From<VfsError> for InitramfsError {
    fn from(err: VfsError) -> Self {
        Self::Vfs(err)
    }
}

impl From<KmodError> for InitramfsError {
    fn from(err: KmodError) -> Self {
        Self::Kmod(err)
    }
}

impl From<UnitError> for InitramfsError {
    fn from(err: UnitError) -> Self {
        Self::System(err)
    }
}

impl From<ElfError> for InitramfsError {
    fn from(err: ElfError) -> Self {
        Self::Elf(err)
    }
}

/// Builder for initramfs generation.
pub struct Initramfs {
    /// Virtual filesystem built for this initramfs.
    vfs: Vfs,
}

impl Initramfs {
    /// Create a new builder.
    pub fn new() -> Result<Self, InitramfsError> {
        let mut vfs = Vfs::default();

        for dir in ROOT_DIRS {
            debug!("Adding default directory: {}", dir);
            vfs.create_dir(dir)?;
        }

        for (src, dest) in ROOT_SYMLINKS {
            debug!("Adding default symlink: {} -> {}", src, dest);
            vfs.create_entry(src, Entry::symlink(dest))?;
        }

        Ok(Initramfs { vfs })
    }

    /// Create a new builder from a configuration.
    pub fn from_config(
        config: &config::Initramfs,
        modules: &[config::Module],
    ) -> Result<Self, InitramfsError> {
        let mut initramfs = Initramfs::new()?;
        initramfs.add_init(&config.init)?;

        if let Some(shutdown) = &config.shutdown {
            initramfs.add_shutdown(shutdown)?;
        }

        let settings = &config.settings;
        let mut kmod = match &settings.kernel_module_path {
            Some(path) => {
                if !path.exists() {
                    let err = io::Error::new(io::ErrorKind::NotFound, path.display().to_string());
                    return Err(InitramfsError::InputOutput(err));
                }

                Kmod::with_directory(path)
            }
            None => Kmod::new(),
        }?;

        for module in modules {
            for binary in &module.binaries {
                initramfs.add_elf(&binary.path)?;
            }

            for spec in &module.files {
                initramfs.add_files(&spec.sources, &spec.destination)?;
            }

            for symlink in &module.symlinks {
                initramfs.add_symlink(&symlink.path, &symlink.target)?;
            }

            for module in &module.kernel_modules {
                match module {
                    config::KernelModule::Name(name) => {
                        initramfs.add_module_from_name(&mut kmod, name)?;
                    }
                    config::KernelModule::Path(path) => {
                        initramfs.add_module_from_path(&mut kmod, path)?;
                    }
                }
            }

            for unit in &module.units {
                initramfs.add_systemd_unit(&unit.name)?;
            }
        }

        Ok(initramfs)
    }

    /// Add the init script from the provided path to the initramfs.
    pub fn add_init(&mut self, path: &Path) -> Result<(), InitramfsError> {
        debug!("Adding init entrypoint: {}", path.display());
        self.add_entrypoint("init", path)?;

        Ok(())
    }

    /// Add the shutdown script, similar to init.
    pub fn add_shutdown(&mut self, path: &Path) -> Result<(), InitramfsError> {
        debug!("Adding shutdown entrypoint: {}", path.display());
        self.add_entrypoint("shutdown", path)?;

        Ok(())
    }

    /// Adds an elf binary to the initramfs, also adding its dynamic dependencies.
    pub fn add_elf(&mut self, path: &Path) -> Result<(), InitramfsError> {
        let path = if path.is_relative() {
            Elf::find_binary(path)?
        } else {
            path.to_path_buf()
        };

        if self.vfs.contains(&path) {
            return Ok(());
        }

        if let Some(parent) = path.parent() {
            self.vfs.create_dir_all(parent)?;
        }

        if !path.exists() {
            error!("Failed to find binary: {}", path.display());

            let err = io::Error::new(io::ErrorKind::NotFound, path.display().to_string());
            return Err(InitramfsError::InputOutput(err));
        }

        debug!("Adding binary: {}", path.display());
        let file = File::open(&path)?;
        let entry = Entry::try_from(file)?;

        self.vfs.create_entry(&path, entry)?;

        for dependency in Elf::linked_libraries(&path)? {
            self.add_elf(&dependency)?;
        }

        Ok(())
    }

    /// Add the filesystem tree from the provided source to the provided destination in the.
    /// initramfs.
    pub fn add_files<P>(&mut self, sources: &[P], destination: &Path) -> Result<(), InitramfsError>
    where
        P: AsRef<Path>,
    {
        debug!("Copying files into {}", destination.display());
        self.vfs.create_dir_all(destination)?;

        for source in sources {
            let source = source.as_ref();

            if !source.exists() {
                error!("Failed to find file: {}", source.display());

                let err = io::Error::new(io::ErrorKind::NotFound, source.display().to_string());
                return Err(InitramfsError::InputOutput(err));
            }

            let metadata = fs::metadata(source)?;
            let ty = metadata.file_type();

            if ty.is_dir() {
                let walk = WalkDir::new(source).min_depth(1);

                for entry in walk {
                    let entry = entry?;

                    let source_path = entry.path();
                    let path = destination.join(
                        source_path
                            .strip_prefix(source)
                            .expect("entry should be under root path"),
                    );

                    if self.vfs.contains(&path) {
                        continue;
                    }

                    let file = File::open(source_path)?;
                    let entry = Entry::try_from(file)?;
                    self.vfs.create_entry(path, entry)?;
                }
            } else {
                let name = source.file_name().expect("path should contain file name");
                let path = destination.join(name);

                if self.vfs.contains(&path) {
                    continue;
                }

                let file = File::open(source)?;
                let entry = Entry::try_from(file)?;
                self.vfs.create_entry(path, entry)?;
            }
        }

        Ok(())
    }

    /// Add a symlink to the initramfs.
    pub fn add_symlink(&mut self, path: &Path, target: &Path) -> Result<(), InitramfsError> {
        if self.vfs.contains(target) {
            return Ok(());
        }

        if let Some(parent) = path.parent() {
            self.vfs.create_dir_all(parent)?;
        }

        debug!("Adding symlink: {} -> {}", path.display(), target.display());

        let entry = Entry::symlink(target);
        self.vfs.create_entry(path, entry)?;

        Ok(())
    }

    /// Add a named kernel module to the initramfs.
    pub fn add_module_from_name(
        &mut self,
        kmod: &mut Kmod,
        name: &str,
    ) -> Result<(), InitramfsError> {
        let module = kmod.module_from_name(name)?;

        debug!("Adding kernel module with name: {}", name);
        self.add_module(kmod, &module)?;

        Ok(())
    }

    /// Add a kernel module to the initramfs from the provided path.
    pub fn add_module_from_path(
        &mut self,
        kmod: &mut Kmod,
        path: &Path,
    ) -> Result<(), InitramfsError> {
        let module = kmod.module_from_path(path)?;

        debug!("Adding kernel module from path: {}", path.display());
        self.add_module(kmod, &module)?;

        Ok(())
    }

    /// Add a systemd unit to the initramfs. This function also adds
    /// binaries used by the unit to the initramfs (ExecStart) and
    /// create relevant symlinks to enable them.
    ///
    /// Service units are 'installed' in the sysinit target and socket
    /// services in the socket target.
    pub fn add_systemd_unit(&mut self, name: &str) -> Result<(), InitramfsError> {
        let Unit {
            path,
            data,
            dependencies,
            binaries,
            install_path,
        } = Unit::from_name(name)?;

        if !self.vfs.contains(&path) {
            debug!("Adding systemd unit: {}", name);

            let entry = Entry::file(data);
            let parent = path.parent().expect("parent directory");

            self.vfs.create_dir_all(parent)?;
            self.vfs.create_entry(path, entry)?;
        }

        // add binaries required by the unit
        for binary in binaries {
            self.add_elf(Path::new(&binary))?;
        }

        // install the unit by adding symlink
        if let Some(path) = install_path {
            let target = Path::new("..").join(name);
            self.add_symlink(&path, &target)?;
        }

        for dependency in dependencies {
            self.add_systemd_unit(&dependency)?;
        }

        Ok(())
    }

    /// Return an archive from this initramfs.
    pub fn into_archive(self) -> Archive {
        Archive::from(self.vfs)
    }

    fn add_entrypoint(&mut self, name: &str, path: &Path) -> Result<(), InitramfsError> {
        let dest = format!("/{name}");
        if self.vfs.contains(&dest) {
            return Ok(());
        }

        if !path.exists() {
            error!("Failed to find {}: {}", name, path.display());

            let err = io::Error::new(io::ErrorKind::NotFound, path.display().to_string());
            return Err(InitramfsError::InputOutput(err));
        }

        let file = File::open(path)?;
        let entry = Entry::try_from(file)?;

        self.vfs.create_entry(dest, entry)?;

        Ok(())
    }

    fn add_module(&mut self, kmod: &mut Kmod, module: &Module) -> Result<(), InitramfsError> {
        // builtin module, nothing to do
        if module.is_builtin() {
            return Ok(());
        }

        // add module dependencies, first
        let debug = module.info()?;
        for name in debug
            .depends()
            .iter()
            .chain(debug.pre_softdeps())
            .chain(debug.post_softdeps())
        {
            let module = kmod.module_from_name(name)?;
            self.add_module(kmod, &module)?;
        }

        // get final path first to avoid reading the file
        // if we have already included it in the vfs
        let path = module.install_path()?;
        if let Some(parent) = path.parent() {
            self.vfs.create_dir_all(parent)?;
        }

        if self.vfs.contains(&path) {
            return Ok(());
        }

        // finally, decompress and create the entry in the vfs
        let compressed = fs::read(module.host_path().expect("module isn't builtin"))?;
        let format = ModuleFormat::from_bytes(&compressed)?;

        let data = uncompress_module(&compressed, &format)?;

        let entry = Entry::file(data);
        self.vfs.create_entry(path, entry)?;

        Ok(())
    }
}

fn uncompress_module(data: &[u8], format: &ModuleFormat) -> Result<Vec<u8>, InitramfsError> {
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

    use std::path::PathBuf;

    #[test]
    fn test_initramfs() {
        let mut binaries = Vec::new();
        let mut files = Vec::new();
        let mut kernel_modules = Vec::new();

        let mut builder = Initramfs::new().unwrap();
        builder.add_init(Path::new("/sbin/init")).unwrap();

        let ls = PathBuf::from("/usr/bin/ls");
        if ls.exists() {
            builder.add_elf(&ls).unwrap();
            binaries.push(config::Binary { path: ls });
        }

        let libc = PathBuf::from("/usr/lib/libc.so.6");
        if libc.exists() {
            builder.add_elf(&libc).unwrap();
            binaries.push(config::Binary { path: libc });
        }

        let hosts = PathBuf::from("/etc/hosts");
        if hosts.exists() {
            builder.add_files(&[&hosts], Path::new("/etc")).unwrap();
            files.push(config::File {
                destination: PathBuf::from("/etc"),
                sources: vec![hosts],
            });
        }

        let udev = PathBuf::from("/usr/lib/udev/rules.d");
        if udev.exists() {
            builder
                .add_files(&[udev.clone()], Path::new("/lib/udev/rules.d"))
                .unwrap();

            files.push(config::File {
                sources: vec![udev],
                destination: PathBuf::from("/lib/udev/rules.d"),
            });
        }

        let mut kmod = Kmod::new().unwrap();
        let btrfs = kmod.module_from_name("btrfs").unwrap();

        if btrfs.host_path().is_some() {
            builder.add_module(&mut kmod, &btrfs).unwrap();
            kernel_modules.push(config::KernelModule::Name("btrfs".to_string()));
        }

        let config = config::Initramfs {
            init: PathBuf::from("/sbin/init"),
            shutdown: None,
            settings: config::Settings::default(),
            modules: Vec::new(),
        };

        let modules = vec![config::Module {
            name: "test".to_string(),
            binaries,
            files,
            kernel_modules,
            symlinks: Vec::new(),
            units: Vec::new(),
        }];

        assert_eq!(
            builder.into_archive(),
            Initramfs::from_config(&config, &modules)
                .unwrap()
                .into_archive(),
        );
    }
}
