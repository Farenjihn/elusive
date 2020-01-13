use crate::config::Initramfs;
use crate::newc::Archive;
use crate::utils;

use flate2::write::GzEncoder;
use flate2::Compression;
use goblin::elf::Elf;
use log::info;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io, os::unix};
use tempfile::TempDir;

const ROOT_DIRS: [&str; 10] = [
    "dev", "etc", "mnt", "proc", "run", "sys", "tmp", "usr/bin", "usr/lib", "var",
];

const ROOT_SYMLINKS: [(&str, &str); 4] = [
    ("bin", "usr/bin"),
    ("lib", "usr/lib"),
    ("lib64", "usr/lib"),
    ("sbin", "usr/bin"),
];

const LIB_LOOKUP_DIRS: [&str; 2] = ["/lib64", "/usr/lib64"];

#[derive(PartialEq, Eq, Hash, Clone)]
pub(crate) struct Entry {
    from: PathBuf,
    to: PathBuf,
}

pub(crate) struct Builder {
    map: HashMap<PathBuf, Entry>,
}

impl Builder {
    pub(crate) fn new() -> io::Result<Self> {
        let builder = Builder {
            map: HashMap::new(),
        };

        Ok(builder)
    }

    pub(crate) fn from_config(
        config: Initramfs,
        kernel_version: Option<String>,
    ) -> io::Result<Self> {
        let mut builder = Builder::new()?;
        builder.add_init(config.init)?;

        if let Some(binaries) = config.bin {
            for binary in binaries {
                builder.add_binary(binary.path)?;
            }
        }

        if let Some(libraries) = config.lib {
            for library in libraries {
                builder.add_library(library.path)?;
            }
        }

        if let Some(modules) = config.module {
            let kernel_version = match kernel_version {
                Some(kernel_version) => kernel_version,
                None => utils::get_kernel_version()?,
            };

            let modules_path = format!("/lib/modules/{}/", kernel_version);
            for module in modules {
                let module_filename = format!("{}.ko", module.name);

                let found = utils::find_file(&[&modules_path], &module_filename);
                let path = match found {
                    Some(path) => path,
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::NotFound,
                            format!("module not found: {}", module.name),
                        ))
                    }
                };

                builder.add_module(path)?;
            }
        }

        Ok(builder)
    }

    pub(crate) fn add_init(&mut self, path: PathBuf) -> io::Result<()> {
        info!("Adding init script: {}", path.to_string_lossy());

        self.map.insert(
            path.clone(),
            Entry {
                from: path,
                to: PathBuf::from("/init"),
            },
        );

        Ok(())
    }

    pub(crate) fn add_module(&mut self, path: PathBuf) -> io::Result<()> {
        if !self.map.contains_key(&path) {
            info!("Adding kernel module: {}", path.to_string_lossy());

            self.map.insert(
                path.clone(),
                Entry {
                    from: path.clone(),
                    to: path.clone(),
                },
            );
        }

        Ok(())
    }

    pub(crate) fn add_binary(&mut self, path: PathBuf) -> io::Result<()> {
        if !self.map.contains_key(&path) {
            info!("Adding binary: {}", path.to_string_lossy());
            return self.add_elf(path, "/usr/bin");
        }

        Ok(())
    }

    pub(crate) fn add_library(&mut self, path: PathBuf) -> io::Result<()> {
        if !self.map.contains_key(&path) {
            info!("Adding library: {}", path.to_string_lossy());
            return self.add_elf(path, "/usr/lib");
        }

        Ok(())
    }

    pub(crate) fn build<P>(self, path: P, ucode: Option<P>) -> io::Result<()>
    where
        P: AsRef<Path>,
    {
        let tmp = TempDir::new()?;
        let tmp_path = tmp.path();

        for dir in &ROOT_DIRS {
            fs::create_dir_all(tmp.path().join(dir))?;
        }

        for link in &ROOT_SYMLINKS {
            unix::fs::symlink(link.1, tmp.path().join(link.0))?;
        }

        for (_, entry) in &self.map {
            let source = &entry.from;
            let dest = tmp_path.join(
                &entry
                    .to
                    .strip_prefix("/")
                    .expect("path should have a leading /"),
            );

            utils::copy_and_chown(&source, dest)?;
        }

        self.depmod(tmp_path)?;

        let path = path.as_ref();
        let mut output_file = utils::maybe_stdout(&path)?;

        if let Some(ucode) = ucode {
            let ucode = ucode.as_ref();
            info!("Adding microcode bundle from: {}", ucode.to_string_lossy());

            let mut file = utils::maybe_stdin(&ucode)?;
            io::copy(&mut file, &mut output_file)?;
        }

        let mut encoder = GzEncoder::new(output_file, Compression::default());

        info!("Writing initramfs to: {}", path.to_string_lossy());
        Archive::from_root(tmp_path, &mut encoder)?;

        Ok(())
    }
}

impl Builder {
    fn add_elf<P>(&mut self, path: PathBuf, dest: P) -> io::Result<()>
    where
        P: AsRef<Path>,
    {
        if !path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("binary not found: {}", path.to_string_lossy()),
            ));
        }

        let bin = fs::read(path.clone())?;
        let elf = Elf::parse(&bin).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "only ELF binaries are supported",
            )
        })?;

        self.add_dependencies(elf)?;

        let filename = match path.file_name() {
            Some(filename) => filename,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("binary path invalid: {}", path.to_string_lossy()),
                ))
            }
        };

        let dest = dest.as_ref().join(filename);
        self.map.insert(
            path.clone(),
            Entry {
                from: path,
                to: dest,
            },
        );

        Ok(())
    }

    fn add_dependencies(&mut self, elf: Elf) -> io::Result<()> {
        let libraries = elf.libraries;

        if !libraries.is_empty() {
            for lib in libraries {
                let path = match LIB_LOOKUP_DIRS
                    .iter()
                    .map(|dir| Path::new(dir).join(lib))
                    .find(|path| path.exists())
                {
                    Some(path) => path,
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::NotFound,
                            "dynamic dependency not found",
                        ))
                    }
                };

                self.add_library(path)?;
            }
        }

        Ok(())
    }

    fn depmod<P>(&self, path: P) -> io::Result<()>
    where
        P: AsRef<Path>,
    {
        Command::new("depmod")
            .args(&[
                "-b",
                path.as_ref()
                    .to_str()
                    .expect("tmpdir path should be valid utf8"),
            ])
            .output()?;

        Ok(())
    }
}
