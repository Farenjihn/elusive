mod config;
mod depend;
mod initramfs;
mod microcode;
mod newc;
mod utils;

use config::Config;

use anyhow::Result;
use clap::{App, AppSettings, Arg, SubCommand};
use env_logger::Env;
use log::warn;
use std::fs;

/// Default path for the config file
const CONFIG_PATH: &str = "/etc/elusive.toml";

/// Entrypoint of the program
fn main() -> Result<()> {
    let env = Env::default().filter_or("RUST_LOG", "info");
    env_logger::init_from_env(env);

    let app = App::new("elusive")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .takes_value(true)
                .global(true)
                .help("Path to the configuration file"),
        )
        .subcommand(
            SubCommand::with_name("initramfs")
                .about("Generate a compressed cpio archive for initramfs")
                .arg(
                    Arg::with_name("kver")
                        .short("k")
                        .long("kver")
                        .help("Kernel version to look up modules")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("ucode")
                        .short("u")
                        .long("ucode")
                        .help("Microcode archive to include")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("output")
                        .short("o")
                        .long("output")
                        .takes_value(true)
                        .required(true)
                        .help("Path where the initramfs will be written"),
                ),
        )
        .subcommand(
            SubCommand::with_name("microcode")
                .about("Generate a cpio archive for your CPU microcode")
                .arg(
                    Arg::with_name("output")
                        .short("o")
                        .long("output")
                        .takes_value(true)
                        .required(true)
                        .help("Path where the microcode archive will be written"),
                ),
        );

    let matches = app.get_matches();

    let config_path = matches.value_of("config").unwrap_or_else(|| CONFIG_PATH);
    let data = fs::read(config_path)?;
    let config: Config = toml::from_slice(&data)?;

    match matches.subcommand() {
        ("initramfs", Some(initramfs)) => {
            let output = initramfs.value_of("output").unwrap();
            let ucode = initramfs.value_of("ucode");
            let kver = initramfs.value_of("kver").map(|kver| kver.into());

            let builder = initramfs::Builder::from_config(config.initramfs, kver)?;
            builder.build(output, ucode)?;
        }
        ("microcode", Some(microcode)) => {
            let output = microcode.value_of("output").unwrap();

            if let Some(microcode) = config.microcode {
                let builder = microcode::Builder::from_config(microcode)?;
                builder.build(output)?;
            } else {
                warn!("No configuration provided for microcode generation");
            }
        }
        (subcommand, _) => unreachable!("unknown subcommand {}", subcommand),
    }

    Ok(())
}
