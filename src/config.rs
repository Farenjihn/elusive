//! Configuration for elusive
//!
//! This module implements the configuration for elusive's initramfs
//! and microcode bundle generation. Configuration is done through a
//! declarative `toml` file that specifies what has to be included in
//! the initramfs or microcode bundle.
//!
//! An example configuration may look like:
//!
//! ```
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
pub(crate) struct Config {
    /// Configuration for initramfs generation
    pub(crate) initramfs: Initramfs,
    /// Configuration for microcode bundle generation
    pub(crate) microcode: Option<Microcode>,
}

/// Initramfs generation configuration
#[derive(Deserialize, Debug)]
pub(crate) struct Initramfs {
    /// Where to find the init (script or binary) for the initramfs
    pub(crate) init: PathBuf,
    /// Kernel modules to add to the initramfs
    pub(crate) module: Option<Vec<Module>>,
    /// Binaries to add to the initramfs
    pub(crate) bin: Option<Vec<Binary>>,
    /// Libraries to add to the initramfs
    pub(crate) lib: Option<Vec<Library>>,
    /// Filesystem trees to copy into the initramfs
    pub(crate) tree: Option<Vec<Tree>>,
}

/// Configuration for a kernel module
#[derive(Deserialize, Debug)]
pub(crate) struct Module {
    /// Name of the kernel module to copy
    pub(crate) name: String,
}

/// Configuration for an executable binary
#[derive(Deserialize, Debug)]
pub(crate) struct Binary {
    /// The path where the binary can be found
    pub(crate) path: PathBuf,
}

/// Configuration for a dynamic library
#[derive(Deserialize, Debug)]
pub(crate) struct Library {
    /// The path where the library can be found
    pub(crate) path: PathBuf,
}

/// Microcode generation configuration
#[derive(Deserialize, Debug)]
pub(crate) struct Microcode {
    /// The path to the AMD specific blobs
    pub(crate) amd: Option<PathBuf>,
    /// The path to the Intel specific blobs
    pub(crate) intel: Option<PathBuf>,
}

/// Configuration for a filesystem tree
#[derive(Deserialize, Debug)]
pub(crate) struct Tree {
    /// The source of the tree to copy
    pub(crate) source: PathBuf,
    /// The destination to copy the tree to
    pub(crate) dest: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_config() {
        let data = fs::read("example.toml").unwrap();
        toml::from_slice::<Config>(&data).unwrap();
    }
}
