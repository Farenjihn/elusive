#![allow(clippy::empty_loop)]

const OK: u32 = 42;

const FAIL_TPM: u32 = 50;

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

fn main() {
    if !Path::new("/dev/tpmrm0").exists() {
        exit_qemu(FAIL_TPM);
    }

    exit_qemu(OK);
}

fn exit_qemu(code: u32) -> ! {
    let mut fd = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/port")
        .expect("open");

    fd.seek(SeekFrom::Start(0xf4)).expect("seek");
    fd.write_all(&code.to_le_bytes()).expect("write");

    loop {}
}
