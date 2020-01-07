mod archive;
mod config;
mod initramfs;
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
                ),
        )
        .get_matches();

    let config_path = matches.value_of("config").unwrap_or_else(|| CONFIG_PATH);
    let data = fs::read(config_path)?;
    let mut config: Config = toml::from_slice(&data)?;

    match matches.subcommand() {
        ("initramfs", Some(initramfs)) => {
            if let Some(kver) = initramfs.value_of("kver") {
                config.initramfs.kernel_version = Some(kver.to_string());
            }

            let builder = initramfs::Builder::from_config(config.initramfs)?;
            builder.build()?;
        }
        (subcommand, _) => unreachable!("unknown subcommand {}", subcommand),
    }

    Ok(())
}
