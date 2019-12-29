use crate::config::Microcode;

use log::info;
use std::fs;
use std::io::Result;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::TempDir;

const INTEL_LOOKUP: &str = "";
const AMD_LOOKUP: &str = "";

#[derive(Debug)]
pub(crate) enum CpuType {
    All,
    Intel,
    Amd,
}

pub(crate) struct Builder {
    tmp: TempDir,
}

impl Builder {
    pub(crate) fn new() -> Result<Self> {
        let tmp = TempDir::new()?;
        Ok(Builder { tmp })
    }

    pub(crate) fn from_config(config: Microcode) -> Result<Self> {
        let mut builder = Builder::new()?;

        let ty = match config.cpu.as_ref() {
            "all" => CpuType::All,
            "intel" => CpuType::Intel,
            "amd" => CpuType::Amd,
            ty => panic!("unknown cpu type for microcode: {}", ty),
        };

        builder.add_cpu(ty)?;
        Ok(builder)
    }

    pub(crate) fn add_cpu(&mut self, ty: CpuType) -> Result<()> {
        info!("Adding microcode for {:?}", ty);
        match ty {
            CpuType::All => todo!(),
            CpuType::Intel => todo!(),
            CpuType::Amd => todo!(),
        }
    }

    pub(crate) fn build<P>(self, output: P) -> Result<()>
    where
        P: Into<PathBuf>,
    {
        let output = output.into();
        info!("Writing microcode cpio to: {}", output.to_string_lossy());

        let path = self.tmp.path();
        let find_cmd = Command::new("find")
            .args(&["."])
            .current_dir(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let cpio_cmd = Command::new("cpio")
            .args(&["-H", "newc", "-o"])
            .current_dir(path)
            .stdin(find_cmd.stdout.expect("find should have output"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()?;

        fs::write(output, cpio_cmd.stdout)?;
        Ok(())
    }
}
