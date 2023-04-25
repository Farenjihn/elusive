//! Microcode bundle generation
//!
//! This module provides an API to help generating a microcode bundle
//! for early loading by the Linux kernel according to its initramfs
//! specification.

use crate::config::Microcode;
use crate::newc::{Archive, Entry, EntryBuilder};

use anyhow::Result;
use log::info;
use std::fs;
use std::path::Path;

/// Path where the blobs will be searched by the Linux kernel
const UCODE_TREE: &str = "/kernel/x86/microcode";

/// Name of the microcode blob for AMD
const AMD_UCODE_NAME: &str = "AuthenticAMD.bin";
/// Name of the microcode blob for Intel
const INTEL_UCODE_NAME: &str = "GenuineIntel.bin";

const DEFAULT_DIR_MODE: u32 = 0o040_000 + 0o755;
const DEFAULT_FILE_MODE: u32 = 0o100_000 + 0o755;

/// Builder pattern for microcode bundle generation
pub struct MicrocodeBundle {
    /// Flag to check if amd ucode was already added
    amd: bool,
    /// Flag to check if intel ucode was already added
    intel: bool,
    /// Entries for the cpio archive
    entries: Vec<Entry>,
}

impl MicrocodeBundle {
    /// Create a new bundle
    pub fn new() -> Result<Self> {
        let mut entries = Vec::new();

        info!("Adding default microcode directory: {}", UCODE_TREE);
        mkdir_all(&mut entries, Path::new(UCODE_TREE));

        Ok(MicrocodeBundle {
            amd: false,
            intel: false,
            entries,
        })
    }

    /// Create a new bundle from a configuration
    pub fn from_config(config: &Microcode) -> Result<Self> {
        let mut bundle = MicrocodeBundle::new()?;

        if let Some(path) = &config.amd_ucode {
            bundle.add_amd_ucode(path)?;
        }

        if let Some(path) = &config.intel_ucode {
            bundle.add_intel_ucode(path)?;
        }

        Ok(bundle)
    }

    /// Bundle amd microcode from the provided path
    pub fn add_amd_ucode(&mut self, path: &Path) -> Result<()> {
        if self.amd {
            return Ok(());
        }

        info!("Bundling AMD microcode");

        let name = Path::new(UCODE_TREE).join(AMD_UCODE_NAME);
        let data = bundle_ucode(path)?;

        let entry = EntryBuilder::file(name, data)
            .mode(DEFAULT_FILE_MODE)
            .build();

        self.entries.push(entry);
        self.amd = true;

        Ok(())
    }

    /// Bundle intel microcode from the provided path
    pub fn add_intel_ucode(&mut self, path: &Path) -> Result<()> {
        if self.intel {
            return Ok(());
        }

        info!("Bundling Intel microcode");

        let name = Path::new(UCODE_TREE).join(INTEL_UCODE_NAME);
        let data = bundle_ucode(path)?;

        let entry = EntryBuilder::file(name, data)
            .mode(DEFAULT_FILE_MODE)
            .build();

        self.entries.push(entry);
        self.intel = true;

        Ok(())
    }

    /// Return an archive from this microcode bundle
    pub fn build(self) -> Archive {
        Archive::new(self.entries)
    }
}

/// Create directory entries by recursively walking the provided path
fn mkdir_all(entries: &mut Vec<Entry>, path: &Path) {
    if path == Path::new("/") {
        return;
    }

    if let Some(parent) = path.parent() {
        mkdir_all(entries, parent);
    }

    let entry = EntryBuilder::directory(path).mode(DEFAULT_DIR_MODE).build();
    entries.push(entry);
}

/// Bundle multiple vendor specific microcode blobs into a single blob
fn bundle_ucode(dir: &Path) -> Result<Vec<u8>> {
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
    fn test_microcode_bundle() -> Result<()> {
        let mut bundle = MicrocodeBundle::new()?;
        let amd = PathBuf::from("/lib/firmware/amd-ucode");

        if amd.exists() {
            bundle.add_amd_ucode(&amd)?;
        }

        bundle.build();

        Ok(())
    }
}
