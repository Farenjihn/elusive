//! Configuration for elusive
//!
//! This module implements the configuration for elusive's initramfs
//! and microcode bundle generation. Configuration is done through a
//! declarative `yaml` file that specifies what has to be included in
//! the initramfs or microcode bundle.
//!
//! An example configuration may look like:
//!
//! ```yaml
//! init: path/to/init/script
//! settings:
//!   decompress_modules: true
//! module:
//!   name: example
//!   binaries:
//!     - /usr/bin/busybox
//!
//! amd_ucode: /lib/firmware/amd-ucode
//! intel_ucode: /lib/firmware/intel-ucode
//! ```

use serde::{Deserialize, Deserializer};
use std::path::PathBuf;

/// Initramfs generation configuration
#[derive(Deserialize, Debug)]
pub struct Initramfs {
    /// Where to find the init script for the initramfs
    pub init: PathBuf,
    /// Where to find the optional shutdown script for the initramfs
    pub shutdown: Option<PathBuf>,
    /// Various flags to tweak generation
    pub settings: Settings,
    /// Optional module for overrides
    pub module: Option<Module>,
}

/// Initramfs generation settings such as various flags
#[derive(Deserialize, Default, Debug)]
pub struct Settings {
    /// Sets whether added kernel modules should be decompressed
    #[serde(default)]
    pub decompress_modules: bool,
    /// Override path where kernel module are searched
    pub boot_module_path: Option<PathBuf>,
    /// Override kernel version for which modules are searched
    pub kernel_release: Option<String>,
}

/// Initramfs configuration module
#[derive(Deserialize, Debug)]
pub struct Module {
    /// Name to refer to this module
    pub name: Option<String>,
    /// Binaries to add to the initramfs
    #[serde(default = "Vec::new")]
    pub binaries: Vec<Binary>,
    /// Filesystem trees to copy into the initramfs
    #[serde(default = "Vec::new")]
    pub files: Vec<File>,
    /// Symlinks to add to the initramfs
    #[serde(default = "Vec::new")]
    pub symlinks: Vec<Symlink>,
    /// Modules to include in the initramfs
    #[serde(default = "Vec::new")]
    pub boot_modules: Vec<BootModule>,
}

/// Configuration for an ELF binary
#[derive(Debug)]
pub struct Binary {
    /// The path where the binary can be found
    pub path: PathBuf,
}

impl<'de> Deserialize<'de> for Binary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{Error, MapAccess, Visitor};
        use std::fmt;

        struct BinaryVisitor;

        impl<'de> Visitor<'de> for BinaryVisitor {
            type Value = Binary;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "a string or a map with one of 'name' or 'path'")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(Binary {
                    path: PathBuf::from(v),
                })
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                match map.next_key::<String>()? {
                    Some(ref key) if key == "path" => Ok(Binary {
                        path: map.next_value()?,
                    }),
                    _ => Err(Error::custom("missing key 'path'".to_string())),
                }
            }
        }

        deserializer.deserialize_any(BinaryVisitor)
    }
}

/// Configuration for a filesystem tree
#[derive(Deserialize, Debug)]
pub struct File {
    /// The destination in the initramfs
    pub destination: PathBuf,
    /// The list of files and directories to copy
    pub sources: Vec<PathBuf>,
}

#[derive(Deserialize, Debug)]
pub struct Symlink {
    /// The path where the symlink will be placed
    pub source: PathBuf,
    /// The file the symlink points to
    pub destination: PathBuf,
}

/// Configuration for a kernel module
#[derive(Debug)]
pub enum BootModule {
    /// Name of the kernel module to include
    Name(String),
    /// Path to the kernel module, useful for out of tree modules
    Path(PathBuf),
}

impl<'de> Deserialize<'de> for BootModule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{Error, MapAccess, Visitor};
        use std::fmt;

        struct BootModuleVisitor;

        impl<'de> Visitor<'de> for BootModuleVisitor {
            type Value = BootModule;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "a string or a map with one of 'name' or 'path'")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(BootModule::Name(v.to_string()))
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                match map.next_key::<String>()? {
                    Some(ref key) if key == "name" => Ok(BootModule::Name(map.next_value()?)),
                    Some(ref key) if key == "path" => Ok(BootModule::Path(map.next_value()?)),
                    _ => Err(Error::custom("missing one of 'name' or 'path'".to_string())),
                }
            }
        }

        deserializer.deserialize_any(BootModuleVisitor)
    }
}

/// Microcode generation configuration
#[derive(Deserialize, Debug)]
pub struct Microcode {
    /// The path to the AMD specific blobs
    pub amd_ucode: Option<PathBuf>,
    /// The path to the Intel specific blobs
    pub intel_ucode: Option<PathBuf>,
}
