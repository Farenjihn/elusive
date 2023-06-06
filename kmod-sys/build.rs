use anyhow::{bail, Result};
use bindgen::Builder;
use std::env;
use std::path::Path;

fn main() -> Result<()> {
    println!("cargo:rustc-link-lib=kmod");

    let builder = Builder::default().header("kmod.h").layout_tests(false);

    let Ok(bindings) = builder.generate() else {
        bail!("failed to generate bindings");
    };

    let dir = env::var("OUT_DIR")?;
    let path = Path::new(&dir).join("bindings.rs");
    bindings.write_to_file(path)?;

    Ok(())
}
