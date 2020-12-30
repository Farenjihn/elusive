//! Various utilities

use anyhow::Result;
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::Path;
use std::{fs, io};

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
        fs::write(path, data)?;
    }

    Ok(())
}
