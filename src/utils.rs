use std::ffi::{CStr, CString, OsStr};
use std::fs::File;
use std::io::{Read, Write};
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::{fs, io};
use walkdir::WalkDir;

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

pub(crate) fn find_file<T, P>(in_dirs: T, filename: &str) -> Option<PathBuf>
where
    T: AsRef<[P]>,
    P: AsRef<Path>,
{
    for dir in in_dirs.as_ref() {
        let found = WalkDir::new(dir).into_iter().find(|entry| {
            let path = entry
                .as_ref()
                .expect("entry should be a valid file")
                .path()
                .to_str()
                .expect("entry should be valid utf8");

            path.contains(&filename)
        });

        if let Some(found) = found {
            return found.ok().map(|entry| entry.into_path());
        }
    }

    None
}

pub(crate) fn copy_and_chown<PSrc, PDst>(source: PSrc, dest: PDst) -> io::Result<()>
where
    PSrc: AsRef<Path>,
    PDst: AsRef<Path>,
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

pub(crate) fn get_kernel_version() -> io::Result<String> {
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
