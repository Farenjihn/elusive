//! Microcode bundle generation
//!
//! This module provides an API to help generating a microcode bundle
//! for early loading by the Linux kernel according to its initramfs
//! specification.

use crate::config::Microcode;
use crate::newc::{Archive, Entry, EntryBuilder};

use anyhow::Result;
use std::fs;
use std::path::{Path};

/// Path where the blobs will be searched by the Linux kernel
const UCODE_TREE: &str = "/kernel/x86/microcode";

/// Name of the microcode blob for AMD
const AMD_UCODE_NAME: &str = "AuthenticAMD.bin";
/// Name of the microcode blob for Intel
const INTEL_UCODE_NAME: &str = "GenuineIntel.bin";

const DEFAULT_DIR_MODE: u32 = 0o040000 + 0o755;
const DEFAULT_FILE_MODE: u32 = 0o100000 + 0o755;

/// Builder pattern for microcode bundle generation
pub struct MicrocodeBundle {
    entries: Vec<Entry>,
}

impl MicrocodeBundle {
    /// Create a new bundle
    pub fn new() -> Result<Self> {
        let mut entries = Vec::new();
        mkdir_all(&mut entries, Path::new(UCODE_TREE));

        Ok(MicrocodeBundle { entries })
    }

    /// Create a new bundle from a configuration
    pub fn from_config(config: Microcode) -> Result<Self> {
        let mut bundle = MicrocodeBundle::new()?;

        if let Some(path) = config.amd {
            bundle.add_amd_ucode(&path)?;
        }

        if let Some(path) = config.intel {
            bundle.add_intel_ucode(&path)?;
        }

        Ok(bundle)
    }

    pub fn add_amd_ucode(&mut self, path: &Path) -> Result<()> {
        let name = Path::new(UCODE_TREE).join(AMD_UCODE_NAME);
        let data = bundle_ucode(&path)?;

        let entry = EntryBuilder::file(name, data)
            .mode(DEFAULT_FILE_MODE)
            .build();

        self.entries.push(entry);

        Ok(())
    }

    pub fn add_intel_ucode(&mut self, path: &Path) -> Result<()> {
        let name = Path::new(UCODE_TREE).join(INTEL_UCODE_NAME);
        let data = bundle_ucode(&path)?;

        let entry = EntryBuilder::file(name, data)
            .mode(DEFAULT_FILE_MODE)
            .build();

        self.entries.push(entry);

        Ok(())
    }

    /// Build the microcode bundle by writing all entries to a temporary directory
    /// and the walking it to create the cpio archive
    pub fn build(self) -> Result<Archive> {
        let archive = Archive::new(self.entries);
        Ok(archive)
    }
}

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
