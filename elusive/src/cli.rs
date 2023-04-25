use crate::config;
use crate::encoder::Encoder;
use crate::initramfs::InitramfsBuilder;
use crate::io::{Input, Output};
use crate::microcode::MicrocodeBundle;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use log::{debug, error, info};
use serde::Deserialize;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::{fs, io};
use thiserror::Error;

/// Default path for the config file
const CONFIG_PATH: &str = "/etc/elusive.yaml";
const CONFDIR_PATH: &str = "/etc/elusive.d";

#[derive(Error, Debug)]
pub enum ConfigurationError {
    #[error("default configuration skipped but no config path specified")]
    SkipWithoutParameter,
    #[error("configuration file is not a file or does not exist: {0}")]
    ExpectedFile(PathBuf),
    #[error("configuration directory is not a directory or does not exist: {0}")]
    ExpectedDirectory(PathBuf),
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    /// Path to the configuration file
    #[clap(short, long)]
    #[clap(global = true)]
    pub config: Option<PathBuf>,
    /// Path to the configuration directory
    #[clap(short = 'C', long)]
    #[clap(global = true)]
    pub confdir: Option<PathBuf>,
    /// Do not read configuration from default paths
    #[clap(long)]
    #[clap(default_value_t = false)]
    #[clap(global = true)]
    pub skip_default_paths: bool,
    #[clap(short, long)]
    #[clap(global = true)]
    /// Encoder to use for compression
    pub encoder: Option<Encoder>,
    #[clap(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Generate a compressed cpio archive to use as initramfs
    Initramfs {
        /// Microcode archive to include
        #[clap(short, long)]
        ucode: Option<PathBuf>,
        /// Path to the kernel module source directory
        #[clap(short, long)]
        modules: Option<PathBuf>,
        /// Kernel release name to overwrite output folder name for kernel modules
        #[clap(short, long)]
        kernel_release: Option<String>,
        /// Path where the initramfs will be written
        #[clap(short, long)]
        output: PathBuf,
    },
    /// Generate a compressed cpio archive for CPU microcode
    Microcode {
        /// Path where the microcode archive will be written
        #[clap(short, long)]
        output: PathBuf,
    },
}

/// Entrypoint of the program
pub fn elusive(args: Args) -> Result<()> {
    let Args {
        config,
        confdir,
        encoder,
        command,
        skip_default_paths,
    } = args;

    let config_path = match (config, skip_default_paths) {
        (Some(path), _) => path,
        (None, false) => PathBuf::from(CONFIG_PATH),
        (None, true) => bail!(ConfigurationError::SkipWithoutParameter),
    };

    debug!("Top-level config file path set to {:?}", config_path);

    let confdir_path = match (confdir, skip_default_paths) {
        (Some(path), _) => Some(path),
        (None, false) => Some(PathBuf::from(CONFDIR_PATH)),
        _ => None,
    };

    debug!("Config module directory path set to {:?}", confdir_path);

    let encoder = encoder.unwrap_or(Encoder::Zstd);

    match command {
        Command::Initramfs {
            ucode,
            modules,
            output,
            kernel_release,
        } => {
            let mut config: config::Initramfs = {
                if !config_path.exists() || !config_path.is_file() {
                    bail!(ConfigurationError::ExpectedFile(config_path));
                }

                debug!("Parsing top-level config file: {:?}", config_path);
                let data = fs::read(config_path)?;
                serde_yaml::from_slice(&data)?
            };

            // override kernel modules path
            if let Some(path) = modules {
                debug!("Overriding kernel module path: {:?}", path);
                config.settings.boot_module_path = Some(path);
            }

            // override kernel release
            if let Some(release) = kernel_release {
                debug!("Overriding kernel release: {:?}", release);
                config.settings.kernel_release = Some(release);
            }

            let mut modules: Vec<config::Module> = Vec::new();
            if let Some(module) = config.module.take() {
                modules.push(module);
            }

            if let Some(confdir_path) = confdir_path {
                if !confdir_path.exists() || !confdir_path.is_dir() {
                    bail!(ConfigurationError::ExpectedDirectory(confdir_path));
                }

                for entry in fs::read_dir(&confdir_path)? {
                    let entry = entry?;
                    let path = entry.path();

                    if path.is_file() {
                        debug!("Parsing module config file: {:?}", path);
                        let data = fs::read(path)?;

                        for document in serde_yaml::Deserializer::from_slice(&data) {
                            let value = serde_yaml::Value::deserialize(document)?;
                            let module = serde_yaml::from_value(value)?;
                            modules.push(module);
                        }
                    }
                }
            }

            info!("Generating initramfs");
            let initramfs = InitramfsBuilder::from_config(&config, &modules)?.build();

            info!("Writing initramfs to: {}", output.display());
            let write = Output::from_path(output)?;
            let mut write = BufWriter::new(write);

            if let Some(path) = ucode {
                info!("Adding microcode bundle from: {}", path.display());

                let read = Input::from_path(path)?;
                let mut read = BufReader::new(read);

                io::copy(&mut read, &mut write)?;
            }

            encoder.encode_archive(initramfs.into_archive(), write)?;
        }
        Command::Microcode { output } => {
            let config: config::Microcode = {
                if !config_path.exists() || !config_path.is_file() {
                    bail!(ConfigurationError::ExpectedFile(config_path));
                }

                let data = fs::read(config_path)?;
                serde_yaml::from_slice(&data)?
            };

            info!("Generating microcode bundle");
            let bundle = MicrocodeBundle::from_config(&config)?;

            info!("Writing microcode cpio to: {}", output.display());
            let write = Output::from_path(output)?;
            let write = BufWriter::new(write);

            encoder.encode_archive(bundle.build(), write)?;
        }
    }

    Ok(())
}
