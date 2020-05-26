use crate::config::Microcode;
use crate::newc::Archive;
use crate::utils;

use anyhow::Result;
use flate2::write::GzEncoder;
use flate2::Compression;
use log::{info, warn};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::{fs, io};
use tempfile::TempDir;

const UCODE_TREE: &str = "kernel/x86/microcode";

const AMD_UCODE_NAME: &str = "AuthenticAMD.bin";
const INTEL_UCODE_NAME: &str = "GenuineIntel.bin";

pub(crate) struct Builder {
    amd: Option<PathBuf>,
    intel: Option<PathBuf>,
}

impl Builder {
    pub(crate) fn new() -> Result<Self> {
        Ok(Builder {
            amd: None,
            intel: None,
        })
    }

    pub(crate) fn from_config(config: Microcode) -> Result<Self> {
        let mut builder = Builder::new()?;

        builder.amd = config.amd;
        builder.intel = config.intel;

        Ok(builder)
    }

    pub(crate) fn build<P>(self, output: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        let output = output.as_ref();
        info!("Writing microcode cpio to: {}", output.to_string_lossy());

        let tmp = TempDir::new()?;
        let tmp_path = tmp.path();

        let ucode_tree = tmp_path.join(UCODE_TREE);
        fs::create_dir_all(&ucode_tree)?;

        if let (None, None) = (&self.amd, &self.intel) {
            warn!("Nothing to do...");
            return Ok(());
        }

        if let Some(amd) = &self.amd {
            add_amd(amd, &ucode_tree)?;
        }

        if let Some(intel) = &self.intel {
            add_intel(intel, &ucode_tree)?;
        }

        let output_file = utils::maybe_stdout(&output)?;
        let mut encoder = GzEncoder::new(output_file, Compression::default());
        Archive::from_root(tmp_path, &mut encoder)?;

        Ok(())
    }
}

fn add_amd<P>(dir: &Path, output: P) -> Result<()>
where
    P: AsRef<Path>,
{
    let output = output.as_ref().join(AMD_UCODE_NAME);
    bundle_ucode(dir, output)
}

fn add_intel<P>(dir: &Path, output: P) -> Result<()>
where
    P: AsRef<Path>,
{
    let output = output.as_ref().join(INTEL_UCODE_NAME);
    bundle_ucode(dir, output)
}

fn bundle_ucode<P>(dir: &Path, output: P) -> Result<()>
where
    P: AsRef<Path>,
{
    let mut output_file = File::create(output.as_ref())?;

    for entry in fs::read_dir(dir)? {
        let entry = entry?;

        if entry.file_type()?.is_file() {
            let mut file = File::open(entry.path())?;
            io::copy(&mut file, &mut output_file)?;
        }
    }

    Ok(())
}
