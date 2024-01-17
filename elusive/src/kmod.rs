//! Wrapper around libkmod for kernel module handling.

#[allow(clippy::wildcard_imports)]
use kmod_sys::*;

use anyhow::{bail, Result};
use std::ffi::CString;
use std::ffi::{CStr, OsStr};
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::{io, ptr};
use thiserror::Error;

const UNKNOWN_MODULE: &str = "unknown";

const MIN_BYTES_LEN: usize = 6;

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
const XZ_MAGIC: [u8; 6] = [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00];
const GZIP_MAGIC: [u8; 2] = [0x1F, 0x8B];

/// Custom error type to represent libkmod failures.
#[derive(Error, Debug)]
pub enum KmodError {
    #[error("failed to create module context")]
    ContextNewFailed,
    #[error("failed to create module from name: {0}")]
    ModuleFromNameFailed(String),
    #[error("failed to create module from path: {0}")]
    ModuleFromPathFailed(PathBuf),
    #[error("failed to get module information: {0}")]
    ModuleGetInfoFailed(String),
    #[error("the data is too small for magic detection")]
    TooSmallForMagic,
    #[error("unknown magic number")]
    UnknownMagic,
}

/// Wrapper handler for libkmod's `kmod_ctx`.
pub struct Kmod {
    dir: PathBuf,
    inner: *mut kmod_ctx,
}

impl Kmod {
    /// Create a new libkmod context.
    pub fn new() -> Result<Self> {
        let release = get_kernel_release()?;
        let dir = Path::new("/lib/modules").join(release);

        Self::with_directory(&dir)
    }

    /// Create a new libkmod context with the specified kernel module directory.
    pub fn with_directory(dir: &Path) -> Result<Self> {
        let cstring = CString::new(dir.as_os_str().as_bytes())?;
        let inner = unsafe { kmod_new(cstring.as_ptr(), ptr::null()) };

        if inner.is_null() {
            bail!(KmodError::ContextNewFailed);
        }

        let kmod = Kmod {
            dir: dir.to_path_buf(),
            inner,
        };

        Ok(kmod)
    }

    /// Get the kernel module directory for this context.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Get a Module with the provided name by searching it in the filesystem.
    pub fn module_from_name<T>(&mut self, name: T) -> Result<Module>
    where
        T: AsRef<str>,
    {
        Module::from_name(self, name)
    }

    /// Get a Module from the provided path which must point to a kernel module.
    pub fn module_from_path<T>(&mut self, path: T) -> Result<Module>
    where
        T: AsRef<Path>,
    {
        Module::from_path(self, path)
    }

    fn as_mut_ptr(&mut self) -> *mut kmod_ctx {
        self.inner
    }
}

impl Drop for Kmod {
    fn drop(&mut self) {
        unsafe {
            let ret = kmod_unref(self.inner);
            assert!(ret.is_null());
        }
    }
}

/// Wrapper handler for libkmod's `kmod_module`.
pub struct Module {
    inner: *mut kmod_module,
}

impl Module {
    fn from_name<T>(ctx: &mut Kmod, name: T) -> Result<Self>
    where
        T: AsRef<str>,
    {
        let name = name.as_ref();
        let cstr = CString::new(name)?;

        let mut list: MaybeUninit<*mut kmod_list> = MaybeUninit::zeroed();

        let inner = unsafe {
            let ret =
                kmod_module_new_from_lookup(ctx.as_mut_ptr(), cstr.as_ptr(), list.as_mut_ptr());

            if ret < 0 {
                bail!(KmodError::ModuleFromNameFailed(name.to_string()));
            }

            let list = list.assume_init();
            let module = kmod_module_get_module(list);

            kmod_module_unref_list(list);
            module
        };

        Ok(Module { inner })
    }

    /// Create a Module from the provided path.
    fn from_path<T>(ctx: &mut Kmod, path: T) -> Result<Self>
    where
        T: AsRef<Path>,
    {
        let path = path.as_ref();
        let data = path.as_os_str().to_os_string().into_vec();
        let cstr = CString::new(data)?;

        let mut inner: MaybeUninit<*mut kmod_module> = MaybeUninit::uninit();

        let inner = unsafe {
            let ret =
                kmod_module_new_from_path(ctx.as_mut_ptr(), cstr.as_ptr(), inner.as_mut_ptr());

            if ret < 0 {
                bail!(KmodError::ModuleFromPathFailed(path.to_path_buf()));
            }

            inner.assume_init()
        };

        Ok(Module { inner })
    }

    /// Get the name of this kernel module.
    pub fn name(&self) -> Option<&str> {
        let cstr = unsafe {
            let name = kmod_module_get_name(self.inner);
            if name.is_null() {
                return None;
            }

            CStr::from_ptr(name)
        };

        cstr.to_str().ok()
    }

    /// Get the path of this kernel module.
    pub fn path(&self) -> Option<&Path> {
        let cstr = unsafe {
            let path = kmod_module_get_path(self.inner);
            if path.is_null() {
                return None;
            }

            CStr::from_ptr(path)
        };

        Some(Path::new(OsStr::from_bytes(cstr.to_bytes())))
    }

    /// Check whether the module is builtin
    pub fn is_builtin(&self) -> bool {
        self.path().is_none()
    }

    /// Get more information on this kernel module.
    pub fn info(&self) -> Result<ModuleInfo> {
        ModuleInfo::new(self)
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        unsafe {
            kmod_module_unref(self.inner);
        }
    }
}

/// Information obtained from a kernel module.
pub struct ModuleInfo {
    /// All aliases for this kernel module.
    aliases: Vec<String>,
    /// All dependencies for this kernel module.
    depends: Vec<String>,
    /// All soft pre-dependencies for this kernel module.
    softpre: Vec<String>,
    /// All soft post-dependencies for this kernel module.
    softpost: Vec<String>,
}

impl ModuleInfo {
    /// Create a new `ModuleInfo` from the provided Module.
    pub fn new(module: &Module) -> Result<Self> {
        let mut list: MaybeUninit<*mut kmod_list> = MaybeUninit::zeroed();

        let mut aliases = Vec::new();
        let mut depends = Vec::new();
        let mut softpre = Vec::new();
        let mut softpost = Vec::new();

        unsafe {
            let ret = kmod_module_get_info(module.inner, list.as_mut_ptr());
            if ret < 0 {
                bail!(KmodError::ModuleGetInfoFailed(
                    module.name().unwrap_or(UNKNOWN_MODULE).to_string()
                ));
            }

            let list = list.assume_init();
            let mut item = list;

            while !item.is_null() {
                let key = kmod_module_info_get_key(item);
                let value = kmod_module_info_get_value(item);

                let key = CStr::from_ptr(key).to_str()?;
                let value = CStr::from_ptr(value);

                match key {
                    "alias" => aliases.push(value.to_str()?.to_string()),
                    "depends" => {
                        for depend in value.to_str()?.split(',') {
                            if !depend.is_empty() {
                                depends.push(depend.to_string());
                            }
                        }
                    }
                    "softdep" => {
                        let value = value.to_str()?;

                        if let Some(softdep) = value.strip_prefix("pre: ") {
                            softpre.push(softdep.to_string());
                        } else if let Some(softdep) = value.strip_prefix("post: ") {
                            softpost.push(softdep.to_string());
                        }
                    }
                    _ => (),
                }

                item = kmod_list_next(list, item);
            }

            kmod_module_info_free_list(list);
        }

        Ok(ModuleInfo {
            aliases,
            depends,
            softpre,
            softpost,
        })
    }

    /// Get a list of aliases for the kernel module.
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }

    /// Get a list of dependencies of the kernel module.
    pub fn depends(&self) -> &[String] {
        &self.depends
    }

    /// Get a list of soft pre-dependencies of the kernel module.
    pub fn pre_softdeps(&self) -> &[String] {
        &self.softpre
    }

    /// Get a list of soft post-dependencies of the kernel module.
    pub fn post_softdeps(&self) -> &[String] {
        &self.softpost
    }
}

/// Enum to represent various compression format for modules.
pub enum ModuleFormat {
    Elf,
    Zstd,
    Xz,
    Gzip,
}

impl ModuleFormat {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < MIN_BYTES_LEN {
            bail!(KmodError::TooSmallForMagic);
        }

        if data[..4] == ELF_MAGIC {
            return Ok(ModuleFormat::Elf);
        }

        if data[..4] == ZSTD_MAGIC {
            return Ok(ModuleFormat::Zstd);
        }

        if data[..6] == XZ_MAGIC {
            return Ok(ModuleFormat::Xz);
        }

        if data[..2] == GZIP_MAGIC {
            return Ok(ModuleFormat::Gzip);
        }

        bail!(KmodError::UnknownMagic);
    }

    pub fn extension(&self) -> &str {
        match self {
            ModuleFormat::Elf => "ko",
            ModuleFormat::Zstd => "ko.zst",
            ModuleFormat::Xz => "ko.xz",
            ModuleFormat::Gzip => "ko.gz",
        }
    }
}

fn get_kernel_release() -> Result<String> {
    let mut utsname: MaybeUninit<libc::utsname> = MaybeUninit::uninit();

    unsafe {
        let ret = libc::uname(utsname.as_mut_ptr());

        if ret < 0 {
            bail!(io::Error::last_os_error());
        }

        let utsname = utsname.assume_init();
        let cstr = CStr::from_ptr(utsname.release.as_ref().as_ptr());

        Ok(cstr.to_str()?.to_string())
    }
}
