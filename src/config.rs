use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Debug)]
pub(crate) struct Config {
    pub(crate) initramfs: Initramfs,
    pub(crate) microcode: Microcode,
}

#[derive(Deserialize, Debug)]
pub(crate) struct Initramfs {
    pub(crate) init: PathBuf,
    pub(crate) module: Option<Vec<Module>>,
    pub(crate) bin: Option<Vec<Binary>>,
    pub(crate) lib: Option<Vec<Library>>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct Module {
    pub(crate) name: String,
}

#[derive(Deserialize, Debug)]
pub(crate) struct Binary {
    pub(crate) path: PathBuf,
}

#[derive(Deserialize, Debug)]
pub(crate) struct Library {
    pub(crate) path: PathBuf,
}

#[derive(Deserialize, Debug)]
pub(crate) struct Microcode {
    pub(crate) cpu: String,
}
