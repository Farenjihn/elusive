mod builder;
mod config;

use builder::Builder;
use config::Config;

use std::error::Error;
use std::fs;

fn main() -> Result<(), Box<dyn Error>> {
    let data = fs::read("example.toml")?;
    let config: Config = toml::from_slice(&data)?;

    let builder = Builder::from_config(config)?;
    builder.build()?;

    Ok(())
}
