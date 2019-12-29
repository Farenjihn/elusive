mod archive;
mod config;
mod initramfs;
mod microcode;
mod newc;

use config::Config;

use clap::{App, AppSettings, Arg, SubCommand};
use env_logger::Env;
use std::error::Error;
use std::fs;

const CONFIG_PATH: &str = "/etc/elusive.toml";

fn main() -> Result<(), Box<dyn Error>> {
    let env = Env::default().filter_or("RUST_LOG", "info");
    env_logger::init_from_env(env);

    let matches = App::new("elusive")
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
        )
        .get_matches();

    let config_path = matches.value_of("config").unwrap_or_else(|| CONFIG_PATH);
    let data = fs::read(config_path)?;
    let config: Config = toml::from_slice(&data)?;

    match matches.subcommand() {
        ("initramfs", Some(initramfs)) => {
            let output = initramfs.value_of("output").unwrap();
            let kver = initramfs.value_of("kver").map(|kver| kver.into());
            let ucode = initramfs.value_of("ucode").map(|ucode| ucode.into());

            let builder = initramfs::Builder::from_config(config.initramfs, kver /*ucode*/)?;
            builder.build(output, ucode)?;
        }
        ("microcode", Some(microcode)) => {
            let output = microcode.value_of("output").unwrap();

            let builder = microcode::Builder::from_config(config.microcode)?;
            builder.build(output)?;
        }
        (subcommand, _) => unreachable!("unknown subcommand {}", subcommand),
    }

    Ok(())
}
