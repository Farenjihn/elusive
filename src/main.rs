use elusive::config::Config;
use elusive::encoder::Encoder;
use elusive::initramfs::InitramfsBuilder;
use elusive::microcode::MicrocodeBundle;
use elusive::utils;

use anyhow::{bail, Result};
// use clap::{App, AppSettings, Arg, SubCommand};
use clap::{AppSettings, Parser, Subcommand};
use env_logger::Env;
use log::{error, info};
use std::fs::File;
use std::io::Read;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::{fs, io};

/// Default path for the config file
const CONFIG_PATH: &str = "/etc/elusive.toml";
const CONFDIR_PATH: &str = "/etc/elusive.d";

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(setting(AppSettings::SubcommandRequiredElseHelp))]
struct Args {
    /// Path to the configuration file
    #[clap(short, long)]
    #[clap(global = true)]
    config: Option<PathBuf>,
    /// Path to the configuration directory
    #[clap(short = 'C', long)]
    #[clap(global = true)]
    confdir: Option<PathBuf>,
    #[clap(short, long)]
    #[clap(global = true)]
    /// Encoder to use for compression
    encoder: Option<Encoder>,
    #[clap(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Generate a compressed cpio archive to use as initramfs
    Initramfs {
        /// Microcode archive to include
        #[clap(short, long)]
        ucode: Option<PathBuf>,
        /// Path to the kernel module source directory
        #[clap(short, long)]
        modules: Option<PathBuf>,
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
#[cfg(not(tarpaulin_include))]
fn main() -> Result<()> {
    let env = Env::default().filter_or("RUST_LOG", "info");
    env_logger::init_from_env(env);

    let args = Args::parse();

    let config_path = args.config.unwrap_or_else(|| PathBuf::from(CONFIG_PATH));
    let confdir_path = args.confdir.unwrap_or_else(|| PathBuf::from(CONFDIR_PATH));

    let mut buf = Vec::new();

    if config_path.exists() && config_path.is_file() {
        File::open(&config_path)?.read_to_end(&mut buf)?;
    }

    if confdir_path.exists() && confdir_path.is_dir() {
        for entry in fs::read_dir(&confdir_path)? {
            let entry = entry?;
            let path = entry.path();

            if path.exists() && path.is_file() {
                File::open(path)?.read_to_end(&mut buf)?;
            }
        }
    }

    if buf.is_empty() {
        bail!(
            "configuration was file or directory was found in {}, {}",
            config_path.display(),
            confdir_path.display(),
        );
    }

    let config: Config = toml::from_slice(&buf)?;
    // use zstd by default
    let encoder = args.encoder.unwrap_or(Encoder::Zstd);

    match args.command {
        Command::Initramfs {
            ucode,
            modules,
            output,
        } => {
            if let Some(config) = config.initramfs {
                let initramfs = InitramfsBuilder::from_config(config, modules.as_deref())?.build();

                info!("Writing initramfs to: {}", output.display());
                let write = utils::file_or_stdout(output)?;
                let mut write = BufWriter::new(write);

                if let Some(path) = ucode {
                    info!("Adding microcode bundle from: {}", path.display());

                    let read = utils::file_or_stdin(path)?;
                    let mut read = BufReader::new(read);

                    io::copy(&mut read, &mut write)?;
                }

                encoder.encode_archive(initramfs.into_archive(), write)?;
            } else {
                error!("No configuration provided for initramfs generation");
                bail!("configuration was empty");
            }
        }
        Command::Microcode { output } => {
            if let Some(config) = config.microcode {
                let bundle = MicrocodeBundle::from_config(config)?;

                info!("Writing microcode cpio to: {}", output.display());
                let write = utils::file_or_stdout(output)?;
                let write = BufWriter::new(write);

                encoder.encode_archive(bundle.build(), write)?;
            } else {
                error!("No configuration provided for microcode generation");
                bail!("configuration was empty");
            }
        }
    }

    Ok(())
}
