use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub initramfs: Initramfs,
}

#[derive(Deserialize, Debug)]
pub struct Initramfs {
    pub path: PathBuf,
    pub init: PathBuf,
    pub modules: Option<Modules>,
    pub bin: Option<Vec<Binary>>,
    pub lib: Option<Vec<Library>>,
}

#[derive(Deserialize, Debug)]
pub struct Modules {
    pub release: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct Binary {
    pub path: PathBuf,
}

#[derive(Deserialize, Debug)]
pub struct Library {
    pub path: PathBuf,
}
