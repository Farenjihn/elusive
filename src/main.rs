use elusive::config::Config;
use elusive::encoder::Encoder;
use elusive::initramfs::InitramfsBuilder;
use elusive::microcode::MicrocodeBundle;
use elusive::utils;

use anyhow::{bail, Result};
use clap::{App, AppSettings, Arg, SubCommand};
use env_logger::Env;
use log::{info, warn};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::{fs, io};

/// Default path for the config file
const CONFIG_PATH: &str = "/etc/elusive.toml";

/// Entrypoint of the program
#[cfg(not(tarpaulin_include))]
fn main() -> Result<()> {
    let env = Env::default().filter_or("RUST_LOG", "info");
    env_logger::init_from_env(env);

    let app = App::new("elusive")
        .version(clap::crate_version!())
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .takes_value(true)
                .global(true)
                .help("Path to the configuration file"),
        )
        .arg(
            Arg::with_name("encoder")
                .short("e")
                .long("encoder")
                .takes_value(true)
                .global(true)
                .help("Encoder to use for compression"),
        )
        .subcommand(
            SubCommand::with_name("initramfs")
                .about("Generate a compressed cpio archive for initramfs")
                .arg(
                    Arg::with_name("ucode")
                        .short("u")
                        .long("ucode")
                        .help("Microcode archive to include")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("modules")
                        .short("m")
                        .long("modules")
                        .help("Path to the kernel module source directory")
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

    let config_path = matches.value_of("config").unwrap_or(CONFIG_PATH);
    if !Path::new(config_path).exists() {
        bail!("configuration file does not exist: {}", config_path);
    }

    let data = fs::read(config_path)?;
    let config: Config = toml::from_slice(&data)?;

    // use zstd by default
    let encoder = match matches.value_of("encoder").unwrap_or("zstd") {
        "none" => Encoder::None,
        "gzip" => Encoder::Gzip,
        "zstd" => Encoder::Zstd,
        other => bail!("unknown encoder: {}", other),
    };

    match matches.subcommand() {
        ("initramfs", Some(initramfs)) => {
            let output = initramfs.value_of("output").unwrap();
            let ucode = initramfs.value_of("ucode");
            let module_dir = initramfs.value_of_os("modules").map(Path::new);

            let initramfs = InitramfsBuilder::from_config(config.initramfs, module_dir)?.build();

            info!("Writing initramfs to: {}", output);
            let write = utils::file_or_stdout(output)?;
            let mut write = BufWriter::new(write);

            if let Some(path) = ucode {
                info!("Adding microcode bundle from: {}", path);

                let read = utils::file_or_stdin(path)?;
                let mut read = BufReader::new(read);

                io::copy(&mut read, &mut write)?;
            }

            encoder.encode_archive(initramfs.into_archive(), write)?;
        }
        ("microcode", Some(microcode)) => {
            let output = microcode.value_of("output").unwrap();

            if let Some(microcode) = config.microcode {
                let bundle = MicrocodeBundle::from_config(microcode)?;
                // let encoded = encoder.encode_archive(bundle.build())?;

                info!("Writing microcode cpio to: {}", output);
                let write = utils::file_or_stdout(output)?;
                let write = BufWriter::new(write);

                encoder.encode_archive(bundle.build(), write)?;
            } else {
                warn!("No configuration provided for microcode generation");
            }
        }
        (subcommand, _) => unreachable!("unknown subcommand {}", subcommand),
    }

    Ok(())
}
