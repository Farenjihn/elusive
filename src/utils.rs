//! Various utilities

use anyhow::{bail, Context, Result};
use log::error;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::{env, io};

/// Allow reading from either a file or standard input
pub fn file_or_stdin<P>(path: P) -> Result<Box<dyn Read>>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();

    let read: Box<dyn Read> = if path == OsStr::new("-") {
        Box::new(io::stdin())
    } else {
        if !path.exists() {
            error!("Input file not found: {}", path.display());
            bail!(io::Error::new(
                io::ErrorKind::NotFound,
                path.to_string_lossy()
            ));
        }

        let file = File::open(path)?;
        Box::new(file)
    };

    Ok(read)
}

/// Allow writing to either a file or standard output
pub fn file_or_stdout<P>(path: P) -> Result<Box<dyn Write>>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();

    let write: Box<dyn Write> = if path == OsStr::new("-") {
        Box::new(io::stdout())
    } else {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir()?.join(path)
        };

        if !absolute
            .parent()
            .context("file has no parent directory")?
            .exists()
        {
            error!(
                "Output file parent directory does not exist: {}",
                absolute.display()
            );

            bail!(io::Error::new(
                io::ErrorKind::NotFound,
                absolute.to_string_lossy()
            ));
        }

        let file = File::create(absolute)?;
        Box::new(file)
    };

    Ok(write)
}
