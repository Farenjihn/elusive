use super::config::Initramfs;

use fs_extra::dir::CopyOptions;
use goblin::elf::Elf;
use std::ffi::CStr;
use std::io::Result;
use std::os::unix;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{fs, io};
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

const LIB_LOOKUP_DIRS: [&str; 6] = [
    "/usr/lib",
    "/usr/lib32",
    "/usr/lib64",
    "/lib",
    "/lib32",
    "/lib64",
];

pub struct Builder {
    path: PathBuf,
    tmp: TempDir,
}

impl Builder {
    pub fn new<P>(path: P) -> Result<Self>
    where
        P: Into<PathBuf>,
    {
        let tmp = TempDir::new()?;

        for dir in &ROOT_DIRS {
            fs::create_dir_all(tmp.path().join(dir))?;
        }

        for link in &ROOT_SYMLINKS {
            unix::fs::symlink(link.1, tmp.path().join(link.0))?;
        }

        let builder = Builder {
            path: path.into(),
            tmp,
        };

        Ok(builder)
    }

    pub fn from_config(config: Initramfs) -> Result<Self> {
        let mut builder = Builder::new(config.path)?;
        builder.add_init(config.init)?;

        if let Some(modules) = config.modules {
            builder.add_modules(modules.release)?;
        }

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

        Ok(builder)
    }

    pub fn add_init(&mut self, path: PathBuf) -> Result<()> {
        if path.exists() {
            fs::copy(path, self.tmp.path().join("init"))?;
        } else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "init not found"));
        }

        Ok(())
    }

    pub fn add_modules(&mut self, release: Option<String>) -> Result<()> {
        let release = match release {
            Some(release) => release,
            None => get_kernel_version()?,
        };

        let path = Path::new("/lib/modules/").join(release);

        if path.exists() {
            fs_extra::dir::copy(
                path,
                self.tmp.path(),
                &CopyOptions {
                    overwrite: false,
                    skip_exist: false,
                    buffer_size: 64000,
                    copy_inside: true,
                    depth: 0,
                },
            )
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "could not copy modules"))?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "could not find kernel modules",
            ));
        }

        Ok(())
    }

    pub fn add_binary(&mut self, path: PathBuf) -> Result<()> {
        if path.exists() {
            let bin = fs::read(path.clone())?;
            let elf = parse_elf(&bin)?;
            let libraries = elf.libraries;

            // lookup and add dynamic libraries
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

            let filename = match path.file_name() {
                Some(filename) => filename,
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "binary path invalid",
                    ))
                }
            };

            let dest = self.tmp.path().join("usr/bin").join(filename);
            fs::copy(path, dest)?;
        } else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "binary not found"));
        }

        Ok(())
    }

    // TODO should it also check for dynamic dependencies ?
    pub fn add_library(&self, path: PathBuf) -> Result<()> {
        if path.exists() {
            let filename = match path.file_name() {
                Some(filename) => filename,
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "binary path invalid",
                    ))
                }
            };

            let dest = self.tmp.path().join("usr/lib").join(filename);
            fs::copy(path, dest)?;
        } else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "library not found"));
        }

        Ok(())
    }

    // TODO replace shelling out with a proper
    // library
    pub fn build(self) -> Result<()> {
        let path = self.tmp.path();
        let find_cmd = Command::new("find")
            .args(&["."])
            .current_dir(path)
            .stdout(Stdio::piped())
            .spawn()?;

        let cpio_cmd = Command::new("cpio")
            .args(&["-H", "newc", "-o"])
            .current_dir(path)
            .stdin(find_cmd.stdout.unwrap())
            .stdout(Stdio::piped())
            .spawn()?;

        let gzip_cmd = Command::new("gzip")
            .current_dir(path)
            .stdin(cpio_cmd.stdout.unwrap())
            .stdout(Stdio::piped())
            .output()?;

        fs::write(self.path, gzip_cmd.stdout)?;
        Ok(())
    }
}

fn parse_elf<'a, T>(data: &'a T) -> Result<Elf<'a>>
where
    T: AsRef<[u8]>,
{
    Elf::parse(data.as_ref()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "only ELF binaries are supported",
        )
    })
}

fn get_kernel_version() -> Result<String> {
    let mut s = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::uname(&mut s) };

    if ret == 0 {
        let version = unsafe { CStr::from_ptr(s.release[..].as_ptr()) }
            .to_string_lossy()
            .into_owned();

        Ok(version)
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "could not read kernel version",
        ))
    }
}
