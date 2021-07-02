//! Various utilities

use anyhow::{bail, Context, Result};
use log::error;
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::Path;
use std::{env, fs, io};

/// Allow reading from either a file or standard input
pub fn read_input<P>(path: P) -> Result<Vec<u8>>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();

    if path == OsStr::new("-") {
        let mut buf = Vec::new();
        io::stdin().read_to_end(&mut buf)?;

        Ok(buf)
    } else {
        if !path.exists() {
            error!("Input file not found: {}", path.display());
            bail!(io::Error::new(
                io::ErrorKind::NotFound,
                path.to_string_lossy()
            ));
        }

        let data = fs::read(path)?;
        Ok(data)
    }
}

/// Allow writing to either a file or standard output
pub fn write_output<P>(path: P, data: &[u8]) -> Result<()>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();

    if path == OsStr::new("-") {
        io::stdout().write_all(data)?;
    } else {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir()?.join(path)
        };

        let parent = absolute.parent().context("no parent directory")?;

        if !parent.exists() {
            error!("Directory does not exist: {}", parent.display());

            bail!(io::Error::new(
                io::ErrorKind::NotFound,
                parent.to_string_lossy()
            ));
        }

        fs::write(path, data)?;
    }

    Ok(())
}
