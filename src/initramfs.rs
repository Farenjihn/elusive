use crate::config::Initramfs;
use crate::newc::Archive;
use crate::utils;

use anyhow::Result;
use flate2::write::GzEncoder;
use flate2::Compression;
use goblin::elf::Elf;
use log::{error, info};
use std::collections::HashMap;
use std::ffi::{CStr, CString, OsStr};
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
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

#[derive(PartialEq, Eq, Hash, Clone)]
pub(crate) struct Entry {
    from: PathBuf,
    to: PathBuf,
}

pub(crate) struct Builder {
    map: HashMap<PathBuf, Entry>,
}

impl Builder {
    pub(crate) fn new() -> Result<Self> {
        let builder = Builder {
            map: HashMap::new(),
        };

        Ok(builder)
    }

    pub(crate) fn from_config(config: Initramfs, kernel_version: Option<String>) -> Result<Self> {
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

    pub(crate) fn add_init(&mut self, path: PathBuf) -> Result<()> {
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

    pub(crate) fn add_module(&mut self, path: PathBuf) -> Result<()> {
        if !self.map.contains_key(&path) {
            info!("Adding kernel module: {}", path.to_string_lossy());

            self.map.insert(
                path.clone(),
                Entry {
                    from: path.clone(),
                    to: path,
                },
            );
        }

        Ok(())
    }

    pub(crate) fn add_binary(&mut self, path: PathBuf) -> Result<()> {
        if !self.map.contains_key(&path) {
            info!("Adding binary: {}", path.to_string_lossy());
            return self.add_elf(path, "/usr/bin");
        }

        Ok(())
    }

    pub(crate) fn add_library(&mut self, path: PathBuf) -> Result<()> {
        if !self.map.contains_key(&path) {
            info!("Adding library: {}", path.to_string_lossy());
            return self.add_elf(path, "/usr/lib");
        }

        Ok(())
    }

    pub(crate) fn add_tree(&mut self, source: PathBuf, dest: PathBuf) -> Result<()> {
        info!("Copying filesystem tree from: {}", source.to_string_lossy());
        self.map.insert(
            source.clone(),
            Entry {
                from: source,
                to: dest,
            },
        );

        Ok(())
    }

    pub(crate) fn build<P>(self, output: P, ucode: Option<P>) -> Result<()>
    where
        P: AsRef<Path>,
    {
        let output = output.as_ref();
        info!("Writing initramfs to: {}", output.to_string_lossy());

        let tmp = TempDir::new()?;
        let tmp_path = tmp.path();

        for dir in &ROOT_DIRS {
            fs::create_dir_all(tmp.path().join(dir))?;
        }

        for link in &ROOT_SYMLINKS {
            unix::fs::symlink(link.1, tmp.path().join(link.0))?;
        }

        for entry in self.map.values() {
            let source = &entry.from;
            let dest = tmp_path.join(
                &entry
                    .to
                    .strip_prefix("/")
                    .expect("path should have a leading /"),
            );

            utils::copy_and_chown(&source, dest)?;
        }

        depmod(tmp_path)?;
        let mut output_file = utils::maybe_stdout(&output)?;

        if let Some(ucode) = ucode {
            let ucode = ucode.as_ref();
            info!("Adding microcode bundle from: {}", ucode.to_string_lossy());

            let mut file = utils::maybe_stdin(&ucode)?;
            io::copy(&mut file, &mut output_file)?;
        }

        let mut encoder = GzEncoder::new(output_file, Compression::default());
        Archive::from_root(tmp_path, &mut encoder)?;

        Ok(())
    }
}

impl Builder {
    fn add_elf<P>(&mut self, path: PathBuf, dest: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        if !path.exists() {
            error!("Failed to find binary: {}", path.to_string_lossy());
            anyhow::bail!("binary not found: {}", path.to_string_lossy());
        }

        let bin = fs::read(path.clone())?;
        let bin = match Elf::parse(&bin) {
            Ok(bin) => bin,
            Err(_) => {
                error!("Failed to parse binary: {}", path.to_string_lossy());
                anyhow::bail!("only ELF binaries are supported");
            }
        };

        self.add_dependencies(bin)?;

        let filename = match path.file_name() {
            Some(filename) => filename,
            None => {
                error!(
                    "Failed to get filename for binary: {}",
                    path.to_string_lossy()
                );
                anyhow::bail!("filename not found in path: {}", path.to_string_lossy());
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

    fn add_dependencies(&mut self, bin: Elf) -> Result<()> {
        let libraries = bin.libraries;

        if !libraries.is_empty() {
            for lib in libraries {
                let name = CString::new(lib).unwrap();
                let mut map: MaybeUninit<*mut link_map> = MaybeUninit::uninit();

                let (handle, ret) = unsafe {
                    let handle = libc::dlopen(name.as_ptr(), libc::RTLD_LAZY);

                    if handle.is_null() {
                        let error = CStr::from_ptr(libc::dlerror())
                            .to_str()
                            .expect("error should be valid utf8");

                        error!("Failed to open handle to dynamic dependency for {}", lib);
                        anyhow::bail!("dlopen failed: {}", error);
                    }

                    let ret = libc::dlinfo(
                        handle,
                        libc::RTLD_DI_LINKMAP,
                        map.as_mut_ptr() as *mut libc::c_void,
                    );

                    (handle, ret)
                };

                if ret < 0 {
                    error!("Failed to get path to dynamic dependency for {}", lib);
                    anyhow::bail!("dlinfo failed");
                }

                let name = unsafe {
                    let map = map.assume_init();
                    CStr::from_ptr((*map).l_name)
                };

                let path = PathBuf::from(OsStr::from_bytes(name.to_bytes()));

                let ret = unsafe { libc::dlclose(handle) };
                if ret < 0 {
                    error!("Failed to close handle to dynamic dependency for {}", lib);
                    anyhow::bail!("dlclose failed");
                }

                self.add_library(path)?;
            }
        }

        Ok(())
    }
}

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

fn depmod<P>(path: P) -> Result<()>
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

#[repr(C)]
struct link_map {
    l_addr: u64,
    l_name: *mut libc::c_char,
    l_ld: *mut libc::c_void,
    l_next: *mut libc::c_void,
    l_prev: *mut libc::c_void,
}
