use anyhow::Result;
use std::ffi::CString;
use std::fs;
use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MAGIC: &[u8] = b"070701";
const TRAILER: &str = "TRAILER!!!";

pub(crate) struct Archive;

impl Archive {
    pub(crate) fn from_root<P, O>(root_dir: P, out: &mut O) -> Result<()>
    where
        P: AsRef<Path> + Clone,
        O: Write,
    {
        let walk = WalkDir::new(root_dir.clone().as_ref())
            .into_iter()
            .skip(1)
            .enumerate();

        let mut buf = Vec::new();
        for (index, dir_entry) in walk {
            let dir_entry = dir_entry?;
            let name = dir_entry
                .path()
                .strip_prefix(root_dir.clone().as_ref())?
                .to_string_lossy();

            let metadata = dir_entry.metadata()?;
            let ty = metadata.file_type();

            let builder = match ty {
                _ if ty.is_dir() => EntryBuilder::directory(&name),
                _ if ty.is_file() => {
                    let file = File::open(dir_entry.path())?;
                    EntryBuilder::file(&name, file)
                }
                _ if ty.is_symlink() => {
                    let path = fs::read_link(dir_entry.path())?;
                    EntryBuilder::symlink(&name, path)
                }
                _ => unreachable!(),
            };

            let entry = builder.with_metadata(metadata).ino(index as u64).build();
            entry.write_to_buf(&mut buf)?;

            out.write_all(&buf)?;
            buf.clear();
        }

        let trailer = EntryBuilder::trailer().ino(0).build();
        trailer.write_to_buf(&mut buf)?;

        out.write_all(&buf)?;

        Ok(())
    }
}

pub(crate) enum EntryType {
    Directory,
    File(File),
    Symlink(PathBuf),
    Trailer,
}

#[derive(Default)]
pub(crate) struct EntryHeader {
    name: String,
    ino: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    nlink: u64,
    mtime: u64,
    dev_major: u64,
    dev_minor: u64,
    rdev_major: u64,
    rdev_minor: u64,
}

impl EntryHeader {
    pub(crate) fn with_name<T>(name: T) -> Self
    where
        T: AsRef<str>,
    {
        EntryHeader {
            name: name.as_ref().to_owned(),
            ..EntryHeader::default()
        }
    }
}

pub(crate) struct Entry {
    ty: EntryType,
    header: EntryHeader,
}

impl Entry {
    fn write_header(&mut self, file_size: usize, buf: &mut Vec<u8>) -> Result<()> {
        let filename = CString::new(self.header.name.clone())?.into_bytes_with_nul();

        buf.reserve(6 + (13 * 8) + filename.len() + file_size);

        buf.extend(MAGIC);
        write!(buf, "{:08x}", self.header.ino)?;
        write!(buf, "{:08x}", self.header.mode)?;
        write!(buf, "{:08x}", self.header.uid)?;
        write!(buf, "{:08x}", self.header.gid)?;
        write!(buf, "{:08x}", self.header.nlink)?;
        write!(buf, "{:08x}", self.header.mtime)?;
        write!(buf, "{:08x}", file_size)?;
        write!(buf, "{:08x}", self.header.dev_major)?;
        write!(buf, "{:08x}", self.header.dev_minor)?;
        write!(buf, "{:08x}", self.header.rdev_major)?;
        write!(buf, "{:08x}", self.header.rdev_minor)?;
        write!(buf, "{:08x}", filename.len())?;
        write!(buf, "{:08x}", 0)?;
        buf.extend(filename);

        Ok(())
    }
}

impl Entry {
    pub(crate) fn write_to_buf(mut self, mut buf: &mut Vec<u8>) -> Result<()> {
        let file_size = match self.ty {
            EntryType::File(ref mut file) => {
                let file_size = file.seek(SeekFrom::End(0))?;
                file.seek(SeekFrom::Start(0))?;

                file_size as usize
            }
            EntryType::Symlink(ref path) => {
                let path_str = path.to_string_lossy();
                path_str.len()
            }
            _ => 0,
        };

        self.write_header(file_size as usize, &mut buf)?;
        pad_buf(&mut buf);

        match self.ty {
            EntryType::File(ref mut file) => {
                file.read_to_end(&mut buf)?;
            }
            EntryType::Symlink(ref path) => {
                buf.extend(path.to_string_lossy().bytes());
            }
            _ => (),
        }

        pad_buf(&mut buf);
        Ok(())
    }
}

pub(crate) struct EntryBuilder {
    entry: Entry,
}

impl EntryBuilder {
    pub(crate) fn directory<T>(name: T) -> Self
    where
        T: AsRef<str>,
    {
        EntryBuilder {
            entry: Entry {
                ty: EntryType::Directory,
                header: EntryHeader::with_name(name),
            },
        }
    }

    pub(crate) fn file<T>(name: T, file: File) -> Self
    where
        T: AsRef<str>,
    {
        EntryBuilder {
            entry: Entry {
                ty: EntryType::File(file),
                header: EntryHeader::with_name(name),
            },
        }
    }

    pub(crate) fn symlink<T>(name: T, path: PathBuf) -> Self
    where
        T: AsRef<str>,
    {
        EntryBuilder {
            entry: Entry {
                ty: EntryType::Symlink(path),
                header: EntryHeader::with_name(name),
            },
        }
    }

    pub(crate) fn trailer() -> Self {
        EntryBuilder {
            entry: Entry {
                ty: EntryType::Trailer,
                header: EntryHeader::with_name(TRAILER),
            },
        }
    }

    pub(crate) fn with_metadata(self, metadata: Metadata) -> Self {
        self.mode(metadata.mode())
            .uid(metadata.uid())
            .gid(metadata.gid())
            .mtime(metadata.mtime() as u64)
    }

    pub(crate) fn ino(mut self, ino: u64) -> Self {
        self.entry.header.ino = ino;
        self
    }

    pub(crate) fn mode(mut self, mode: u32) -> Self {
        self.entry.header.mode = mode;
        self
    }

    pub(crate) fn uid(mut self, uid: u32) -> Self {
        self.entry.header.uid = uid;
        self
    }

    pub(crate) fn gid(mut self, gid: u32) -> Self {
        self.entry.header.gid = gid;
        self
    }

    pub(crate) fn mtime(mut self, mtime: u64) -> Self {
        self.entry.header.mtime = mtime;
        self
    }

    pub(crate) fn build(self) -> Entry {
        self.entry
    }
}

pub fn pad_buf(buf: &mut Vec<u8>) {
    let rem = buf.len() % 4;
    if rem != 0 {
        buf.resize(buf.len() + (4 - rem), 0);
    }
}
