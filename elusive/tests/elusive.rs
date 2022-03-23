use elusive::cli;
use elusive::cli::{Args, Command};

use anyhow::Result;
use std::path::PathBuf;

fn dev_null() -> PathBuf {
    PathBuf::from("/dev/null")
}

#[test]
fn test_microcode() -> Result<()> {
    let args = Args {
        config: Some(PathBuf::from("config/ucode.toml")),
        confdir: Some(dev_null()),
        encoder: None,
        command: Command::Microcode { output: dev_null() },
    };

    cli::elusive(args)?;

    Ok(())
}

#[test]
fn test_basic() -> Result<()> {
    let args = Args {
        config: Some(PathBuf::from("config/basic.toml")),
        confdir: Some(dev_null()),
        encoder: None,
        command: Command::Initramfs {
            ucode: None,
            modules: None,
            output: dev_null(),
        },
    };

    cli::elusive(args)?;

    Ok(())
}

#[test]
fn test_systemd() -> Result<()> {
    let args = Args {
        config: Some(PathBuf::from("config/systemd.toml")),
        confdir: Some(PathBuf::from("config/systemd.d/")),
        encoder: None,
        command: Command::Initramfs {
            ucode: None,
            modules: None,
            output: dev_null(),
        },
    };

    cli::elusive(args)?;

    Ok(())
}
