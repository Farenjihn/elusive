use crate::config::Initramfs;
use crate::newc::Archive;
use crate::utils;

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

        if let Some(trees) = config.tree {
            for tree in trees {
                builder.add_tree(tree.source, tree.dest)?;
            }
        }

        if let Some(modules) = config.module {
            for module in modules {
                let path = builder.modinfo(module.name, &kernel_version)?;
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
                    to: path,
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

    pub(crate) fn add_tree(&mut self, source: PathBuf, dest: PathBuf) -> io::Result<()> {
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

    pub(crate) fn build<P>(self, output: P, ucode: Option<P>) -> io::Result<()>
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

        self.depmod(tmp_path)?;
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
                let name = CString::new(lib).unwrap();
                let mut map: MaybeUninit<*mut link_map> = MaybeUninit::uninit();

                let (handle, ret) = unsafe {
                    let handle = libc::dlopen(name.as_ptr(), libc::RTLD_LAZY);
                    let ret = dlinfo(
                        handle,
                        RTLD_DI_LINKMAP,
                        map.as_mut_ptr() as *mut libc::c_void,
                    );

                    (handle, ret)
                };

                if ret < 0 {
                    error!("Failed to get path to dynamic dependency for {}", lib);
                    return Err(io::Error::new(io::ErrorKind::Other, "dlinfo failed"));
                }

                let name = unsafe {
                    let map = map.assume_init();
                    CStr::from_ptr((*map).l_name)
                };

                let path = PathBuf::from(OsStr::from_bytes(name.to_bytes()));

                let ret = unsafe { libc::dlclose(handle) };
                if ret < 0 {
                    error!("Failed to close handle to dynamic dependency for {}", lib);
                    return Err(io::Error::new(io::ErrorKind::Other, "dlclose failed"));
                }

                self.add_library(path)?;
            }
        }

        Ok(())
    }

    fn modinfo<T>(&self, name: T, kernel_version: &Option<String>) -> io::Result<PathBuf>
    where
        T: AsRef<str>,
    {
        let mut command = Command::new("modinfo");

        if let Some(version) = kernel_version {
            command.args(&["-k", &version]);
        }

        command.args(&["-n", name.as_ref()]).output().map(|output| {
            let path = std::str::from_utf8(&output.stdout)
                .expect("modinfo output should be valid utf8")
                .trim_end();

            path.into()
        })
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

// FIXME: remove this once a new version of libc is released

use libc::{c_int, c_void};

const RTLD_DI_LINKMAP: c_int = 2;

#[repr(C)]
struct link_map {
    l_addr: u64,
    l_name: *mut libc::c_char,
    l_ld: *mut libc::c_void,
    l_next: *mut libc::c_void,
    l_prev: *mut libc::c_void,
}

#[link(name = "dl")]
extern "C" {
    pub fn dlinfo(handle: *mut c_void, request: c_int, info: *mut c_void) -> c_int;
}
