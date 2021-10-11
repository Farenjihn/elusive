use anyhow::{bail, Result};
use kmod_sys::*;
use std::collections::HashMap;
use std::ffi::CString;
use std::ffi::{CStr, OsStr};
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::{io, ptr};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum KmodError {
    #[error("failed to create module context")]
    ContextNewFailed,
    #[error("failed to create module from name: {0}")]
    ModuleFromNameFailed(String),
    #[error("failed to create module from path: {0}")]
    ModuleFromPathFailed(PathBuf),
    #[error("a module with the same name was already added: {0}")]
    ModuleNameCollision(String),
}

pub struct Kmod {
    dir: PathBuf,
    inner: *mut kmod_ctx,
    modules: HashMap<String, Rc<Module>>,
}

impl Kmod {
    pub fn new() -> Result<Self> {
        let release = get_kernel_release()?;
        let dir = Path::new("/lib/modules").join(release);

        Self::with_directory(&dir)
    }

    pub fn with_directory(dir: &Path) -> Result<Self> {
        let cstring = CString::new(dir.as_os_str().as_bytes())?;
        let inner = unsafe { kmod_new(cstring.as_ptr(), ptr::null()) };

        if inner.is_null() {
            bail!(KmodError::ContextNewFailed);
        } else {
            Ok(Kmod {
                dir: dir.to_path_buf(),
                inner,
                modules: HashMap::new(),
            })
        }
    }

    pub fn as_mut_ptr(&mut self) -> *mut kmod_ctx {
        self.inner
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn module_from_name<T>(&mut self, name: T) -> Result<Rc<Module>>
    where
        T: AsRef<str>,
    {
        let module = Module::from_name(self, name)?;
        self.module(module)
    }

    pub fn module_from_path<T>(&mut self, path: T) -> Result<Rc<Module>>
    where
        T: AsRef<Path>,
    {
        let module = Module::from_path(self, path)?;
        self.module(module)
    }
}

impl Kmod {
    fn module(&mut self, module: Module) -> Result<Rc<Module>> {
        let name = module.name()?;
        let name = name.to_string();

        if self.modules.contains_key(&name) {
            bail!(KmodError::ModuleNameCollision(name));
        }

        let module = Rc::new(module);
        self.modules.insert(name, module.clone());

        Ok(module)
    }
}

impl Drop for Kmod {
    fn drop(&mut self) {
        unsafe {
            kmod_unref(self.inner);
        }
    }
}

pub struct Module {
    inner: *mut kmod_module,
}

impl Module {
    pub fn name(&self) -> Result<&str> {
        let cstr = unsafe {
            let name = kmod_module_get_name(self.inner);
            CStr::from_ptr(name)
        };

        let name = cstr.to_str()?;
        Ok(name)
    }

    pub fn path(&self) -> &Path {
        let cstr = unsafe {
            let path = kmod_module_get_path(self.inner);
            CStr::from_ptr(path)
        };

        Path::new(OsStr::from_bytes(cstr.to_bytes()))
    }
}

impl Module {
    fn from_name<T>(ctx: &mut Kmod, name: T) -> Result<Self>
    where
        T: AsRef<str>,
    {
        let name = name.as_ref();
        let cstr = CString::new(name)?;

        let mut inner: MaybeUninit<*mut kmod_module> = MaybeUninit::uninit();

        let inner = unsafe {
            let ret =
                kmod_module_new_from_name(ctx.as_mut_ptr(), cstr.as_ptr(), inner.as_mut_ptr());

            if ret < 0 {
                bail!(KmodError::ModuleFromNameFailed(name.to_string()));
            }

            inner.assume_init()
        };

        Ok(Module { inner })
    }

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
