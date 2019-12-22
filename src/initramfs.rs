use super::config::Initramfs;

use goblin::elf::Elf;
use std::ffi::CStr;
use std::io::Result;
use std::mem::MaybeUninit;
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

const LIB_LOOKUP_DIRS: [&str; 2] = ["/lib64", "/usr/lib64"];

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

        if let Some(modules) = config.module {
            let kernel_version = match config.kernel_version {
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

    pub fn add_init(&mut self, path: PathBuf) -> Result<()> {
        if path.exists() {
            fs::copy(path, self.tmp.path().join("init"))?;
        } else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "init not found"));
        }

        Ok(())
    }

    pub fn add_module(&mut self, kernel_version: &str, name: String) -> Result<()> {
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

        let source = Path::new(output);
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

        fs::copy(
            source,
            target.join(source.file_name().expect("path should have filename")),
        )?;

        Ok(())
    }

    pub fn add_binary(&mut self, path: PathBuf) -> Result<()> {
        self.add_elf(path, "usr/bin")
    }

    pub fn add_library(&self, path: PathBuf) -> Result<()> {
        self.add_elf(path, "usr/lib")
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
            .stdin(find_cmd.stdout.expect("find should have output"))
            .stdout(Stdio::piped())
            .spawn()?;

        let gzip_cmd = Command::new("gzip")
            .current_dir(path)
            .stdin(cpio_cmd.stdout.expect("cpio should have output"))
            .stdout(Stdio::piped())
            .output()?;

        fs::write(self.path, gzip_cmd.stdout)?;
        Ok(())
    }
}

impl Builder {
    fn add_elf<P>(&self, path: PathBuf, dest: P) -> Result<()>
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
            fs::copy(path, dest)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("binary not found: {}", path.to_string_lossy()),
            ));
        }

        Ok(())
    }

    fn add_dependencies(&self, elf: Elf) -> Result<()> {
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

    fn depmod(&self) -> Result<()> {
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
