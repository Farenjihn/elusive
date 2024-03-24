//! Microcode bundle generation
//!
//! This module provides an API to help generating a microcode bundle
//! for early loading by the Linux kernel according to its initramfs
//! specification.

use crate::config::Microcode;
use crate::newc::Archive;
use crate::vfs::{Entry, Vfs, VfsError};

use log::info;
use std::path::Path;
use std::{fs, io};

/// Path where the blobs will be searched by the Linux kernel.
const UCODE_TREE: &str = "/kernel/x86/microcode";

/// Name of the microcode blob for AMD.
const AMD_UCODE_NAME: &str = "AuthenticAMD.bin";
/// Name of the microcode blob for Intel.
const INTEL_UCODE_NAME: &str = "GenuineIntel.bin";

/// Custom error type for microcode archive generation.
#[derive(thiserror::Error, Debug)]
pub enum MicrocodeError {
    #[error("i/o error: {0}")]
    InputOutput(io::Error),
    #[error("vfs error: {0}")]
    Vfs(VfsError),
}

impl From<io::Error> for MicrocodeError {
    fn from(err: io::Error) -> Self {
        Self::InputOutput(err)
    }
}

impl From<VfsError> for MicrocodeError {
    fn from(err: VfsError) -> Self {
        Self::Vfs(err)
    }
}

/// Builder pattern for microcode bundle generation.
pub struct MicrocodeBundle {
    /// Flag to check if amd ucode was already added.
    amd: bool,
    /// Flag to check if intel ucode was already added.
    intel: bool,
    /// Virtual filesystem built for this microcode archive.
    vfs: Vfs,
}

impl MicrocodeBundle {
    /// Create a new bundle.
    pub fn new() -> Result<Self, MicrocodeError> {
        let mut vfs = Vfs::new();

        info!("Adding default microcode directory: {}", UCODE_TREE);
        vfs.create_dir_all(UCODE_TREE)?;

        Ok(MicrocodeBundle {
            amd: false,
            intel: false,
            vfs,
        })
    }

    /// Create a new bundle from a configuration.
    pub fn from_config(config: &Microcode) -> Result<Self, MicrocodeError> {
        let mut bundle = MicrocodeBundle::new()?;

        if let Some(path) = &config.amd_ucode {
            bundle.add_amd_ucode(path)?;
        }

        if let Some(path) = &config.intel_ucode {
            bundle.add_intel_ucode(path)?;
        }

        Ok(bundle)
    }

    /// Bundle amd microcode from the provided path.
    pub fn add_amd_ucode(&mut self, path: &Path) -> Result<(), MicrocodeError> {
        if self.amd {
            return Ok(());
        }

        info!("Bundling AMD microcode");

        let data = bundle_ucode(path)?;
        let entry = Entry::file(data);

        let path = Path::new(UCODE_TREE).join(AMD_UCODE_NAME);
        self.vfs.create_entry(path, entry)?;
        self.amd = true;

        Ok(())
    }

    /// Bundle intel microcode from the provided path.
    pub fn add_intel_ucode(&mut self, path: &Path) -> Result<(), MicrocodeError> {
        if self.intel {
            return Ok(());
        }

        info!("Bundling Intel microcode");

        let data = bundle_ucode(path)?;
        let entry = Entry::file(data);

        let path = Path::new(UCODE_TREE).join(INTEL_UCODE_NAME);
        self.vfs.create_entry(path, entry)?;
        self.intel = true;

        Ok(())
    }

    /// Return an archive from this microcode bundle..
    #[must_use]
    pub fn into_archive(self) -> Archive {
        Archive::from(self.vfs)
    }
}

/// Bundle multiple vendor specific microcode blobs into a single blob.
fn bundle_ucode(dir: &Path) -> Result<Vec<u8>, MicrocodeError> {
    let mut data = Vec::new();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;

        if entry.file_type()?.is_file() {
            data.extend(fs::read(entry.path())?);
        }
    }

    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_microcode_bundle() -> Result<(), MicrocodeError> {
        let mut bundle = MicrocodeBundle::new()?;
        let amd = PathBuf::from("/lib/firmware/amd-ucode");

        if amd.exists() {
            bundle.add_amd_ucode(&amd)?;
        }

        let _ = bundle.into_archive();
        Ok(())
    }
}
