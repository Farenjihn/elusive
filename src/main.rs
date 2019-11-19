mod config;
mod initramfs;

use config::Config;

use clap::{App, AppSettings, Arg, SubCommand};
use std::error::Error;
use std::fs;

const CONFIG_PATH: &str = "/etc/elusive.toml";

fn main() -> Result<(), Box<dyn Error>> {
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
                .about("Generate a compressed cpio archive for initramfs"),
        )
        .get_matches();

    let config_path = matches.value_of("config").unwrap_or_else(|| CONFIG_PATH);
    let data = fs::read(config_path)?;
    let config: Config = toml::from_slice(&data)?;

    match matches.subcommand() {
        ("initramfs", Some(_)) => {
            let builder = initramfs::Builder::from_config(config.initramfs)?;
            builder.build()?;
        }
        (subcommand, _) => unreachable!("unknown subcommand {}", subcommand),
    }

    Ok(())
}
