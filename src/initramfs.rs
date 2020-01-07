use crate::archive;
use crate::config::Initramfs;

use flate2::write::GzEncoder;
use flate2::Compression;
use goblin::elf::Elf;
use log::info;
use std::collections::HashSet;
use std::ffi::{CStr, CString, OsStr};
use std::fs::File;
use std::io::{Read, Write};
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

pub(crate) struct Builder {
    tmp: TempDir,
    set: HashSet<PathBuf>,
}

impl Builder {
    pub(crate) fn new() -> io::Result<Self> {
        let tmp = TempDir::new()?;

        for dir in &ROOT_DIRS {
            fs::create_dir_all(tmp.path().join(dir))?;
        }

        for link in &ROOT_SYMLINKS {
            unix::fs::symlink(link.1, tmp.path().join(link.0))?;
        }

        let builder = Builder {
            tmp,
            set: HashSet::new(),
        };

        Ok(builder)
    }

    pub(crate) fn from_config(
        config: Initramfs,
        kernel_version: Option<String>,
    ) -> io::Result<Self> {
        let mut builder = Builder::new()?;
        builder.add_init(config.init)?;

        if let Some(modules) = config.module {
            let kernel_version = match kernel_version {
                Some(kernel_version) => kernel_version,
                None => get_kernel_version()?,
            };

            for module in modules {
                builder.add_module(&kernel_version, module.name)?;
            }

            builder.depmod()?;
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

    pub(crate) fn add_init(&mut self, path: PathBuf) -> io::Result<()> {
        info!("Adding init script: {}", path.to_string_lossy());

        if path.exists() {
            let dest = self.tmp.path().join("init");
            copy_and_chown(path, dest)?;
        } else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "init not found"));
        }

        Ok(())
    }

    pub(crate) fn add_module(&mut self, kernel_version: &str, name: String) -> io::Result<()> {
        info!("Adding kernel module: {}", name);

        let modules_path = format!("/lib/modules/{}/", kernel_version);
        let module_filename = format!("{}.ko.*", name);

        let find_cmd = Command::new("find")
            .args(&[&modules_path, "-name", &module_filename])
            .stdout(Stdio::piped())
            .output()?;

        if find_cmd.stdout.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("module not found: {}", name),
            ));
        }

        let output = std::str::from_utf8(&find_cmd.stdout)
            .expect("find should return a valid utf8 string")
            .trim();

        let source = PathBuf::from(output);
        let target = self.tmp.path().join(
            source
                .parent()
                .expect("path should have parent")
                .strip_prefix("/")
                .expect("parent should have a leading slash"),
        );

        if !target.exists() {
            fs::create_dir_all(&target)?;
        }

        let dest = target.join(source.file_name().expect("path should have filename"));
        copy_and_chown(source, dest)?;

        Ok(())
    }

    pub(crate) fn add_binary(&mut self, path: PathBuf) -> io::Result<()> {
        if !self.set.contains(&path) {
            info!("Adding binary: {}", path.to_string_lossy());
            self.set.insert(path.clone());

            return self.add_elf(path, "usr/bin");
        }

        Ok(())
    }

    pub(crate) fn add_library(&mut self, path: PathBuf) -> io::Result<()> {
        if !self.set.contains(&path) {
            info!("Adding library: {}", path.to_string_lossy());
            self.set.insert(path.clone());

            return self.add_elf(path, "usr/lib");
        }

        Ok(())
    }

    pub(crate) fn build<P>(self, path: P, ucode: Option<P>) -> io::Result<()>
    where
        P: Into<PathBuf>,
    {
        let path = path.into();
        let mut output_file = maybe_stdout(&path)?;

        if let Some(ucode) = ucode {
            let ucode = ucode.into();
            info!("Adding microcode bundle from: {}", ucode.to_string_lossy());

            let mut file = maybe_stdin(&ucode)?;
            io::copy(&mut file, &mut output_file)?;
        }

        let mut encoder = GzEncoder::new(output_file, Compression::default());

        info!("Writing initramfs to: {}", path.to_string_lossy());
        let tmp_root = self.tmp.path();
        archive::write_archive(tmp_root, &mut encoder)?;

        Ok(())
    }
}

impl Builder {
    fn add_elf<P>(&mut self, path: PathBuf, dest: P) -> io::Result<()>
    where
        P: Into<PathBuf>,
    {
        if path.exists() {
            let bin = fs::read(path.clone())?;
            let elf = parse_elf(&bin)?;
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

            let dest = self.tmp.path().join(dest.into()).join(filename);
            copy_and_chown(path, dest)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("binary not found: {}", path.to_string_lossy()),
            ));
        }

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

    fn depmod(&self) -> io::Result<()> {
        Command::new("depmod")
            .args(&[
                "-b",
                self.tmp
                    .path()
                    .to_str()
                    .expect("tmpdir path should be valid utf8"),
            ])
            .output()?;

        Ok(())
    }
}

fn parse_elf<'a, T>(data: &'a T) -> io::Result<Elf<'a>>
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

fn copy_and_chown<P>(source: P, dest: P) -> io::Result<()>
where
    P: AsRef<Path>,
{
    fs::copy(&source, &dest)?;

    unsafe {
        let c_dest = CString::new(dest.as_ref().as_os_str().as_bytes()).unwrap();
        libc::chown(c_dest.as_ptr(), 0, 0);
    }

    Ok(())
}

fn maybe_stdin<P>(path: P) -> io::Result<Box<dyn Read>>
where
    P: AsRef<Path>,
{
    if path.as_ref() == OsStr::new("-") {
        Ok(Box::new(io::stdin()))
    } else {
        Ok(Box::new(File::open(&path)?))
    }
}

fn maybe_stdout<P>(path: P) -> io::Result<Box<dyn Write>>
where
    P: AsRef<Path>,
{
    if path.as_ref() == OsStr::new("-") {
        Ok(Box::new(io::stdout()))
    } else {
        Ok(Box::new(File::create(&path)?))
    }
}

fn get_kernel_version() -> io::Result<String> {
    let version = unsafe {
        let mut buf = MaybeUninit::uninit();
        let ret = libc::uname(buf.as_mut_ptr());

        if ret == 0 {
            let buf = buf.assume_init();
            CStr::from_ptr(buf.release[..].as_ptr())
        } else {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "could not read kernel version",
            ));
        }
    };

    Ok(version.to_string_lossy().into_owned())
}
