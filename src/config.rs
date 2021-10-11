//! Configuration for elusive
//!
//! This module implements the configuration for elusive's initramfs
//! and microcode bundle generation. Configuration is done through a
//! declarative `toml` file that specifies what has to be included in
//! the initramfs or microcode bundle.
//!
//! An example configuration may look like:
//!
//! ```toml
//! [initramfs]
//! init = "init"
//!
//! [[initramfs.bin]]
//! path = "/bin/busybox"
//!
//! [[initramfs.lib]]
//! path = "/lib64/ld-linux-x86-64.so.2"
//!
//! [microcode]
//! amd = "/lib/firmware/amd-ucode"
//! intel = "/lib/firmware/intel-ucode"
//! ```

use serde::Deserialize;
use std::path::PathBuf;

/// Top-level configuration structure
#[derive(Deserialize, Debug)]
pub struct Config {
    /// Configuration for initramfs generation
    pub initramfs: Initramfs,
    /// Configuration for microcode bundle generation
    pub microcode: Option<Microcode>,
}

/// Initramfs generation configuration
#[derive(Deserialize, Debug)]
pub struct Initramfs {
    /// Where to find the init script for the initramfs
    pub init: PathBuf,
    /// Where to find the optional shutdown script for the initramfs
    pub shutdown: Option<PathBuf>,
    /// Binaries to add to the initramfs
    pub bin: Option<Vec<Binary>>,
    /// Libraries to add to the initramfs
    pub lib: Option<Vec<Library>>,
    /// Filesystem trees to copy into the initramfs
    pub tree: Option<Vec<Tree>>,
    /// Modules to include in the initramfs
    pub module: Option<Vec<Module>>,
}

/// Configuration for an executable binary
#[derive(Deserialize, Debug)]
pub struct Binary {
    /// The path where the binary can be found
    pub path: PathBuf,
}

/// Configuration for a dynamic library
#[derive(Deserialize, Debug)]
pub struct Library {
    /// The path where the library can be found
    pub path: PathBuf,
}

/// Microcode generation configuration
#[derive(Deserialize, Debug)]
pub struct Microcode {
    /// The path to the AMD specific blobs
    pub amd: Option<PathBuf>,
    /// The path to the Intel specific blobs
    pub intel: Option<PathBuf>,
}

/// Configuration for a filesystem tree
#[derive(Deserialize, Debug)]
pub struct Tree {
    /// The destination in the initramfs
    pub path: PathBuf,
    /// The list of files and directories to copy
    pub copy: Vec<PathBuf>,
}

/// Configuration for a kernel module
#[derive(Deserialize, Debug)]
pub struct Module {
    /// Name of the kernel module to include
    pub name: Option<String>,
    /// Path to the kernel module, useful for out of tree modules
    pub path: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    use anyhow::Result;
    use std::fs;

    #[test]
    fn test_config() -> Result<()> {
        let data = fs::read("example.toml")?;
        toml::from_slice::<Config>(&data)?;

        Ok(())
    }
}
