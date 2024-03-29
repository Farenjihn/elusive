#![deny(clippy::all)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::must_use_candidate)]

#[doc(hidden)]
pub mod cli;

pub mod config;
pub mod elf;
pub mod encoder;
pub mod initramfs;
pub mod io;
pub mod kmod;
pub mod microcode;
pub mod newc;
pub mod systemd;
pub mod vfs;

mod search;
