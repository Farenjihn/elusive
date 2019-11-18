use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub initramfs: Initramfs,
    pub bin: Option<Vec<Binary>>,
    pub lib: Option<Vec<Library>>,
}

#[derive(Deserialize, Debug)]
pub struct Initramfs {
    pub path: PathBuf,
    pub init: PathBuf,
    pub modules: bool,
}

#[derive(Deserialize, Debug)]
pub struct Binary {
    pub path: PathBuf,
}

#[derive(Deserialize, Debug)]
pub struct Library {
    pub path: PathBuf,
}
