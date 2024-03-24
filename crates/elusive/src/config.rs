//! Configuration for initramfs and microcode generation.
//!
//! This module implements the configuration for elusive's initramfs
//! and microcode bundle generation. Configuration is done through a
//! declarative `yaml` file that specifies what has to be included in
//! the initramfs or microcode bundle.
//!
//! An example top-level configuration may look as follows:
//!
//! ```yaml
//! amd_ucode: /lib/firmware/amd-ucode
//! intel_ucode: /lib/firmware/intel-ucode
//!
//! init: path/to/init/script
//! modules:
//!   - base
//!   - kmod
//!   - blk-ahci
//!   - blk-ata
//!   - blk-nvme
//!   - blk-virtio
//!   - fs-ext4
//!   - fs-btrfs
//!   - device-mapper
//!   - systemd
//!   - systemd-crypt
//!   - systemd-tpm
//!   - systemd-udev
//!   - usb-hid
//! ```
//!
//! Then, a module configuration may look like this:
//!
//! ```yaml
//! name: base
//! kernel_modules:
//!   - crc32
//!   - crc32c
//!   - crc32-generic
//!   - crc32c-generic
//! binaries:
//!   - blkid
//!   - busybox
//!   - lsblk
//!   - mount
//!   - switch_root
//!   - umount
//! files:
//!   - sources:
//!       - contrib/files/etc/group
//!       - contrib/files/etc/initrd-release
//!       - contrib/files/etc/nsswitch.conf
//!       - contrib/files/etc/passwd
//!       - contrib/files/etc/shadow
//!     destination: /etc
//! symlinks:
//!   - path: /usr/bin/sh
//!     target: busybox
//! ```
//!
//! For more examples, see the `contrib` directory in the repository.

use serde::{Deserialize, Deserializer};
use std::path::PathBuf;

/// Microcode generation configuration.
#[derive(Deserialize, Debug)]
pub struct Microcode {
    /// The path to the AMD specific blobs.
    pub amd_ucode: Option<PathBuf>,
    /// The path to the Intel specific blobs.
    pub intel_ucode: Option<PathBuf>,
}

/// Initramfs generation configuration.
#[derive(Deserialize, Debug)]
pub struct Initramfs {
    /// Where to find the init script for the initramfs.
    pub init: PathBuf,
    /// Where to find the optional shutdown script for the initramfs.
    pub shutdown: Option<PathBuf>,
    /// Various flags to tweak generation.
    #[serde(default)]
    pub settings: Settings,
    /// Enabled modules.
    pub modules: Vec<String>,
}

/// Initramfs generation settings such as various flags.
#[derive(Deserialize, Default, Debug)]
pub struct Settings {
    /// Override path where kernel module are searched.
    pub kernel_module_path: Option<PathBuf>,
}

/// Initramfs configuration module.
#[derive(Deserialize, Debug)]
pub struct Module {
    /// Name to refer to this module.
    pub name: String,
    /// Binaries to add to the initramfs.
    #[serde(default = "Vec::new")]
    pub binaries: Vec<Binary>,
    /// Filesystem trees to copy into the initramfs.
    #[serde(default = "Vec::new")]
    pub files: Vec<File>,
    /// Symlinks to add to the initramfs.
    #[serde(default = "Vec::new")]
    pub symlinks: Vec<Symlink>,
    /// Modules to include in the initramfs.
    #[serde(default = "Vec::new")]
    pub kernel_modules: Vec<KernelModule>,
    /// Units (systemd) to include in the initramfs.
    #[serde(default = "Vec::new")]
    pub units: Vec<Unit>,
}

/// Configuration for an ELF binary.
#[derive(Debug)]
pub struct Binary {
    /// The path where the binary can be found.
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

/// Configuration for a filesystem tree.
#[derive(Deserialize, Debug)]
pub struct File {
    /// The list of files and directories to copy.
    pub sources: Vec<PathBuf>,
    /// The destination in the initramfs.
    pub destination: PathBuf,
}

/// Configuration for a symbolic link.
#[derive(Deserialize, Debug)]
pub struct Symlink {
    /// The path where the symlink will be placed.
    pub path: PathBuf,
    /// The file the symlink points to.
    pub target: PathBuf,
}

/// Configuration for a kernel module.
#[derive(Debug)]
pub enum KernelModule {
    /// Name of the kernel module to include.
    Name(String),
    /// Path to the kernel module, useful for out of tree modules.
    Path(PathBuf),
}

impl<'de> Deserialize<'de> for KernelModule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{Error, MapAccess, Visitor};
        use std::fmt;

        struct BootModuleVisitor;

        impl<'de> Visitor<'de> for BootModuleVisitor {
            type Value = KernelModule;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "a string or a map with one of 'name' or 'path'")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(KernelModule::Name(v.to_string()))
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                match map.next_key::<String>()? {
                    Some(ref key) if key == "name" => Ok(KernelModule::Name(map.next_value()?)),
                    Some(ref key) if key == "path" => Ok(KernelModule::Path(map.next_value()?)),
                    _ => Err(Error::custom("missing one of 'name' or 'path'".to_string())),
                }
            }
        }

        deserializer.deserialize_any(BootModuleVisitor)
    }
}

/// Configuration for a systemd unit.
#[derive(Debug)]
pub struct Unit {
    /// Name of the unit to include.
    pub name: String,
}

impl<'de> Deserialize<'de> for Unit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{Error, MapAccess, Visitor};
        use std::fmt;

        struct UnitVisitor;

        impl<'de> Visitor<'de> for UnitVisitor {
            type Value = Unit;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "a string or a map with one of 'name' or 'path'")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(Unit {
                    name: v.to_string(),
                })
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                match map.next_key::<String>()? {
                    Some(ref key) if key == "name" => Ok(Unit {
                        name: map.next_value()?,
                    }),
                    _ => Err(Error::custom("missing key 'name'".to_string())),
                }
            }
        }

        deserializer.deserialize_any(UnitVisitor)
    }
}
