use crate::newc::EntryBuilder;

use log::error;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::{fs, io};
use walkdir::WalkDir;

pub(crate) fn write_archive<P, O>(root_dir: P, mut out: &mut O) -> io::Result<()>
where
    P: Into<PathBuf> + Clone,
    O: Write,
{
    let root_dir = Arc::new(root_dir.into());

    WalkDir::new(root_dir.clone().as_ref())
        .into_iter()
        .skip(1)
        .enumerate()
        .fold(&mut out, |out, (index, entry)| {
            let res = entry
                .map_err(|err| Box::new(err) as Box<dyn std::error::Error>)
                .and_then(|entry| {
                    let name = entry
                        .path()
                        .strip_prefix(root_dir.clone().as_ref())?
                        .to_string_lossy();

                    let metadata = entry.metadata()?;
                    let ty = metadata.file_type();

                    let builder = match ty {
                        _ if ty.is_dir() => EntryBuilder::directory(&name),
                        _ if ty.is_file() => {
                            let file = File::open(entry.path())?;
                            EntryBuilder::file(&name, file)
                        }
                        _ if ty.is_symlink() => {
                            let path = fs::read_link(entry.path())?;
                            EntryBuilder::symlink(&name, path)
                        }
                        _ => unreachable!(),
                    };

                    let entry = builder.with_metadata(metadata).ino(index as u64).build();
                    Ok(entry)
                });

            match res {
                Ok(entry) => {
                    let mut buf = Vec::new();
                    let res = entry
                        .write_to_buf(&mut buf)
                        .and_then(|_| out.write_all(&buf));

                    if let Err(err) = res {
                        error!("Error while writing entry: {}", err);
                    }
                }
                Err(err) => {
                    error!("Error while walking tmpdir: {}", err);
                }
            }

            out
        });

    let trailer = EntryBuilder::trailer().ino(0).build();
    let mut buf = Vec::new();

    trailer.write_to_buf(&mut buf)?;
    out.write_all(&buf)?;

    Ok(())
}
