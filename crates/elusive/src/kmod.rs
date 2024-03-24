//! Wrapper around libkmod for kernel module handling.

#[allow(clippy::wildcard_imports)]
use kmod_sys::*;

use std::ffi::CString;
use std::ffi::{CStr, OsStr};
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::{ffi, io, ptr, str};

const UNKNOWN_MODULE: &str = "unknown";

const MAGIC_ELF: [u8; 4] = [0x7F, b'E', b'L', b'F'];

const MAGIC_XZ: [u8; 6] = [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00];
const MAGIC_GZ: [u8; 2] = [0x1F, 0x8B];
const MAGIC_ZSTD: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

const FORMAT_MIN_BYTES_LEN: usize = 6;

/// Custom error type to represent libkmod failures.
#[derive(thiserror::Error, Debug)]
pub enum KmodError {
    #[error("i/o error: {0}")]
    InputOutput(io::Error),
    #[error("failed to convert to utf8: {0}")]
    Utf8Conversion(str::Utf8Error),
    #[error("spurious interior nul byte: {0}")]
    InteriorNulByte(ffi::NulError),
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
    #[error("the {0} directory must point to kernel modules (e.g. /lib/modules/$(uname -r) )")]
    BadDirectory(PathBuf),
    #[error("the module is already built-in")]
    ModuleBuiltIn,
}

impl From<io::Error> for KmodError {
    fn from(err: io::Error) -> Self {
        Self::InputOutput(err)
    }
}

impl From<str::Utf8Error> for KmodError {
    fn from(err: str::Utf8Error) -> Self {
        Self::Utf8Conversion(err)
    }
}

impl From<ffi::NulError> for KmodError {
    fn from(err: ffi::NulError) -> Self {
        Self::InteriorNulByte(err)
    }
}

/// Wrapper handler for libkmod's `kmod_ctx`.
pub struct Kmod {
    kernel_release: Rc<String>,
    ctx: *mut kmod_ctx,
}

impl Kmod {
    /// Create a new libkmod context.
    pub fn new() -> Result<Self, KmodError> {
        let kernel_release = get_kernel_release()?;
        let ctx = Self::kmod_init_ctx(&Path::new("/usr/lib/modules").join(&kernel_release))?;

        Ok(Kmod {
            kernel_release: Rc::new(kernel_release),
            ctx,
        })
    }

    /// Create a new libkmod context with the specified kernel module directory.
    pub fn with_directory(dir: &Path) -> Result<Self, KmodError> {
        if !Path::exists(&dir.join("kernel")) {
            return Err(KmodError::BadDirectory(dir.into()));
        }

        let filename = dir.file_name().expect("path it not root");

        let kernel_release = filename.to_string_lossy().to_string();
        let ctx = Self::kmod_init_ctx(dir)?;

        let kmod = Kmod {
            kernel_release: Rc::new(kernel_release),
            ctx,
        };

        Ok(kmod)
    }

    /// Get the kernel release for modules in the context directory.
    pub fn kernel_release(&self) -> &str {
        &self.kernel_release
    }

    /// Get a Module with the provided name by searching it in the filesystem.
    pub fn module_from_name<T>(&mut self, name: T) -> Result<Module, KmodError>
    where
        T: AsRef<str>,
    {
        Module::from_name(self, name)
    }

    /// Get a Module from the provided path which must point to a kernel module.
    pub fn module_from_path<T>(&mut self, path: T) -> Result<Module, KmodError>
    where
        T: AsRef<Path>,
    {
        Module::from_path(self, path)
    }

    fn kmod_init_ctx(dir: &Path) -> Result<*mut kmod_ctx, KmodError> {
        let cstring = CString::new(dir.as_os_str().as_bytes())?;
        let inner = unsafe { kmod_new(cstring.as_ptr(), ptr::null()) };

        if inner.is_null() {
            return Err(KmodError::ContextNewFailed);
        }

        Ok(inner)
    }

    fn as_mut_ptr(&mut self) -> *mut kmod_ctx {
        self.ctx
    }
}

impl Drop for Kmod {
    fn drop(&mut self) {
        unsafe {
            let ret = kmod_unref(self.ctx);
            assert!(ret.is_null());
        }
    }
}

/// Wrapper handler for libkmod's `kmod_module`.
pub struct Module {
    kernel_release: Rc<String>,
    inner: *mut kmod_module,
}

impl Module {
    fn from_name<T>(ctx: &mut Kmod, name: T) -> Result<Self, KmodError>
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
                return Err(KmodError::ModuleFromNameFailed(name.to_string()));
            }

            let list = list.assume_init();
            let module = kmod_module_get_module(list);

            kmod_module_unref_list(list);
            module
        };

        Ok(Module {
            kernel_release: ctx.kernel_release.clone(),
            inner,
        })
    }

    /// Create a Module from the provided path.
    fn from_path<T>(ctx: &mut Kmod, path: T) -> Result<Self, KmodError>
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
                return Err(KmodError::ModuleFromPathFailed(path.to_path_buf()));
            }

            inner.assume_init()
        };

        Ok(Module {
            kernel_release: ctx.kernel_release.clone(),
            inner,
        })
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

    /// Get the host path of this kernel module.
    pub fn host_path(&self) -> Option<&Path> {
        let cstr = unsafe {
            let path = kmod_module_get_path(self.inner);
            if path.is_null() {
                return None;
            }

            CStr::from_ptr(path)
        };

        Some(Path::new(OsStr::from_bytes(cstr.to_bytes())))
    }

    /// Get the install path for this kernel module.
    pub fn install_path(&self) -> Result<PathBuf, KmodError> {
        let Some(host_path) = self.host_path() else {
            return Err(KmodError::ModuleBuiltIn);
        };

        let inner_path = host_path
            .components()
            .skip_while(|component| component.as_os_str() != "kernel");

        let mut install_path = PathBuf::from("/usr/lib/modules").join(self.kernel_release.as_ref());

        install_path.extend(inner_path);
        install_path.set_file_name(self.name().expect("module has a name"));
        install_path.set_extension("ko");

        Ok(install_path)
    }

    /// Check whether the module is builtin
    pub fn is_builtin(&self) -> bool {
        self.host_path().is_none()
    }

    /// Get more information on this kernel module.
    pub fn info(&self) -> Result<ModuleInfo, KmodError> {
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
    pub fn new(module: &Module) -> Result<Self, KmodError> {
        let mut list: MaybeUninit<*mut kmod_list> = MaybeUninit::zeroed();

        let mut aliases = Vec::new();
        let mut depends = Vec::new();
        let mut softpre = Vec::new();
        let mut softpost = Vec::new();

        unsafe {
            let ret = kmod_module_get_info(module.inner, list.as_mut_ptr());
            if ret < 0 {
                return Err(KmodError::ModuleGetInfoFailed(
                    module.name().unwrap_or(UNKNOWN_MODULE).to_string(),
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
                    // TODO: firmware ?
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
    pub fn from_bytes(data: &[u8]) -> Result<Self, KmodError> {
        if data.len() < FORMAT_MIN_BYTES_LEN {
            return Err(KmodError::TooSmallForMagic);
        }

        if data[..4] == MAGIC_ELF {
            return Ok(ModuleFormat::Elf);
        }

        if data[..4] == MAGIC_ZSTD {
            return Ok(ModuleFormat::Zstd);
        }

        if data[..6] == MAGIC_XZ {
            return Ok(ModuleFormat::Xz);
        }

        if data[..2] == MAGIC_GZ {
            return Ok(ModuleFormat::Gzip);
        }

        Err(KmodError::UnknownMagic)
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

fn get_kernel_release() -> Result<String, KmodError> {
    let mut utsname: MaybeUninit<libc::utsname> = MaybeUninit::uninit();

    unsafe {
        let ret = libc::uname(utsname.as_mut_ptr());
        if ret < 0 {
            return Err(io::Error::last_os_error().into());
        }

        let utsname = utsname.assume_init();
        let cstr = CStr::from_ptr(utsname.release.as_ref().as_ptr());

        Ok(cstr.to_str().expect("kernel ").to_string())
    }
}
