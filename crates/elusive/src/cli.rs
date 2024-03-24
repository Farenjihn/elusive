use crate::config;
use crate::encoder::Encoder;
use crate::initramfs::Initramfs;
use crate::io::{Input, Output};
use crate::microcode::MicrocodeBundle;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use log::{debug, error, info};
use std::collections::BTreeMap;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::{fs, io};

const DEFAULT_CONFIG_PATH: &str = "/etc/elusive.yaml";
const DEFAULT_CONFDIR_PATHS: &[&str] = &["/etc/elusive.d", "/usr/share/elusive.d"];

#[derive(thiserror::Error, Debug)]
pub enum ConfigurationError {
    #[error("default paths skipped but no fallback paths specified")]
    SkipWithoutParameter,
    #[error("configuration requires a module named '{0}' but none was found")]
    UnknownModule(String),
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
    pub confdir: Option<Vec<PathBuf>>,
    /// Do not read configuration from default paths
    #[clap(long)]
    #[clap(default_value_t = false)]
    #[clap(global = true)]
    pub skip_default_paths: bool,
    /// Encoder to use for compression
    #[clap(short, long)]
    #[clap(global = true)]
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
        // /// Kernel release name to overwrite output folder name for kernel modules
        // #[clap(short, long)]
        // kernel_release: Option<String>,
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
        confdir: confdirs,
        encoder,
        command,
        skip_default_paths,
    } = args;

    let config_path = match (config, skip_default_paths) {
        (Some(path), _) => path,
        (None, false) => PathBuf::from(DEFAULT_CONFIG_PATH),
        (None, true) => bail!(ConfigurationError::SkipWithoutParameter),
    };

    let default_confdirs = DEFAULT_CONFDIR_PATHS.iter().map(PathBuf::from).collect();
    let confdir_paths = match (confdirs, skip_default_paths) {
        (Some(paths), false) => paths.into_iter().chain(default_confdirs).collect(),
        (Some(paths), true) => paths,
        (None, false) => default_confdirs,
        (None, true) => bail!(ConfigurationError::SkipWithoutParameter),
    };

    debug!("Config file path set to {:?}", config_path);
    debug!("Module directory paths set to {:?}", confdir_paths);

    let encoder = encoder.unwrap_or(Encoder::Zstd);

    match command {
        Command::Initramfs {
            ucode,
            modules,
            output,
            // kernel_release,
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
                config.settings.kernel_module_path = Some(path);
            }

            // parse all available modules
            let mut modules = BTreeMap::new();
            for path in confdir_paths {
                if !path.exists() || !path.is_dir() {
                    bail!(ConfigurationError::ExpectedDirectory(path));
                }

                for entry in fs::read_dir(&path)? {
                    let entry = entry?;
                    let path = entry.path();

                    if path.is_file() {
                        debug!("Parsing module config file: {:?}", path);
                        let data = fs::read(path)?;
                        let module = serde_yaml::from_slice::<config::Module>(&data)?;

                        modules.insert(module.name.clone(), module);
                    }
                }
            }

            // check all selected modules are present
            let mut selected: Vec<config::Module> = Vec::new();
            for name in &config.modules {
                let module = modules
                    .remove(name.as_str())
                    .context(ConfigurationError::UnknownModule(name.clone()))?;

                selected.push(module)
            }

            info!("Generating initramfs");
            let archive = Initramfs::from_config(&config, &selected)?.into_archive();
            let serialized = archive.serialize()?;

            info!("Writing initramfs to: {}", output.display());
            let output = Output::from_path(output)?;
            let mut output = BufWriter::new(output);

            if let Some(path) = ucode {
                info!("Adding microcode bundle from: {}", path.display());

                let read = Input::from_path(path)?;
                let mut read = BufReader::new(read);

                io::copy(&mut read, &mut output)?;
            }

            encoder.encode(&serialized, output)?;
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
            let archive = MicrocodeBundle::from_config(&config)?.into_archive();
            let serialized = archive.serialize()?;

            info!("Writing microcode cpio to: {}", output.display());
            let output = Output::from_path(output)?;
            let output = BufWriter::new(output);

            encoder.encode(&serialized, output)?;
        }
    }

    Ok(())
}
