use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::{fs, io};

pub(crate) fn maybe_stdin<P>(path: P) -> io::Result<Box<dyn Read>>
where
    P: AsRef<Path>,
{
    if path.as_ref() == OsStr::new("-") {
        Ok(Box::new(io::stdin()))
    } else {
        Ok(Box::new(File::open(&path)?))
    }
}

pub(crate) fn maybe_stdout<P>(path: P) -> io::Result<Box<dyn Write>>
where
    P: AsRef<Path>,
{
    if path.as_ref() == OsStr::new("-") {
        Ok(Box::new(io::stdout()))
    } else {
        Ok(Box::new(File::create(&path)?))
    }
}

pub(crate) fn copy_and_chown<S, D>(source: S, dest: D) -> io::Result<()>
where
    S: AsRef<Path>,
    D: AsRef<Path>,
{
    let parent = dest.as_ref().parent().expect("path should have a parent");

    fs::create_dir_all(parent)?;
    fs::copy(&source, &dest)?;

    unsafe {
        let c_dest = CString::new(dest.as_ref().as_os_str().as_bytes()).unwrap();
        libc::chown(c_dest.as_ptr(), 0, 0);
    }

    Ok(())
}
