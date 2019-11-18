use super::config::Config;

use goblin::elf::Elf;
use goblin::Object;
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

    pub fn from_config(config: Config) -> Result<Self> {
        let mut builder = Builder::new(config.initramfs.path)?;
        builder.add_init(config.initramfs.init)?;

        if config.initramfs.modules {
            builder.add_modules()?;
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

    pub fn add_modules(&mut self) -> Result<()> {
        let mut s = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::uname(&mut s) };

        if ret == 0 {
            let version = unsafe { CStr::from_ptr(s.release[..].as_ptr()) }
                .to_string_lossy()
                .into_owned();

            let path = Path::new("/lib/modules/").join(version);

            if path.exists() {
                fs::copy(path, self.tmp.path())?;
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "could not find kernel modules",
                ));
            }
        } else {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "could not read kernel version",
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
            .args(&["-9"])
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
    // TODO handle error correctly
    let object = Object::parse(data.as_ref()).unwrap();
    match object {
        Object::Elf(elf) => Ok(elf),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only ELF binaries are supported",
        )),
    }
}
