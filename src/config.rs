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
    pub kernel_version: Option<String>,
    pub module: Option<Vec<Module>>,
    pub bin: Option<Vec<Binary>>,
    pub lib: Option<Vec<Library>>,
}

#[derive(Deserialize, Debug)]
pub struct Module {
    pub name: String,
}

#[derive(Deserialize, Debug)]
pub struct Binary {
    pub path: PathBuf,
}

#[derive(Deserialize, Debug)]
pub struct Library {
    pub path: PathBuf,
}
