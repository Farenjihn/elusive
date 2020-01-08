use crate::newc::EntryBuilder;

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::{fs, io};
use walkdir::WalkDir;

pub(crate) fn write_archive<P, O>(root_dir: P, out: &mut O) -> io::Result<()>
where
    P: AsRef<Path> + Clone,
    O: Write,
{
    let walk = WalkDir::new(root_dir.clone().as_ref())
        .into_iter()
        .skip(1)
        .enumerate();

    for (index, dir_entry) in walk {
        let dir_entry = dir_entry?;
        let name = dir_entry
            .path()
            .strip_prefix(root_dir.clone().as_ref())
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?
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

        let mut data = Vec::new();
        let entry = builder.with_metadata(metadata).ino(index as u64).build();
        entry.write_to_buf(&mut data)?;

        out.write_all(&data)?;
    }

    let mut buf = Vec::new();

    let trailer = EntryBuilder::trailer().ino(0).build();
    trailer.write_to_buf(&mut buf)?;

    out.write_all(&buf)?;

    Ok(())
}
