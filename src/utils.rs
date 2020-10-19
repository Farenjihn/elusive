//! Various utilities

use anyhow::Result;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::{fs, io};

/// Allow reading from either a file or standard input
pub(crate) fn maybe_stdin<P>(path: P) -> Result<Box<dyn Read>>
where
    P: AsRef<Path>,
{
    if path.as_ref() == OsStr::new("-") {
        Ok(Box::new(io::stdin()))
    } else {
        Ok(Box::new(File::open(&path)?))
    }
}

/// Allow writing to either a file or standard output
pub(crate) fn maybe_stdout<P>(path: P) -> Result<Box<dyn Write>>
where
    P: AsRef<Path>,
{
    if path.as_ref() == OsStr::new("-") {
        Ok(Box::new(io::stdout()))
    } else {
        Ok(Box::new(File::create(&path)?))
    }
}

/// Copy files
pub(crate) fn copy_files<S, D>(source: S, dest: D) -> Result<()>
where
    S: AsRef<Path>,
    D: AsRef<Path>,
{
    let source = source.as_ref();
    let dest = dest.as_ref();

    let parent = dest.parent().expect("path should have a parent");
    fs::create_dir_all(parent)?;

    let metadata = fs::metadata(&source)?;
    let ty = metadata.file_type();

    if ty.is_file() {
        fs::copy(&source, &dest)?;
    } else if ty.is_dir() {
        let options = fs_extra::dir::CopyOptions {
            overwrite: true,
            skip_exist: false,
            buffer_size: 64000,
            copy_inside: true,
            content_only: true,
            depth: 0,
        };

        fs_extra::dir::copy(&source, &dest, &options)?;
    }

    Ok(())
}
