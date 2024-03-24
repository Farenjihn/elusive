//! I/O utilities.

use log::error;
use std::ffi::OsStr;
use std::path::Path;
use std::{env, fs, io};

/// Allow reading from either a file or standard input.
pub enum Input {
    Stdin(io::Stdin),
    File(fs::File),
}

impl Input {
    /// Create an Input from a provided path. If the path is '-'.
    /// then the Input will read from standard input.
    pub fn from_path<T>(path: T) -> Result<Self, io::Error>
    where
        T: AsRef<Path>,
    {
        let path = path.as_ref();

        if path == OsStr::new("-") {
            return Ok(Input::Stdin(io::stdin()));
        }

        if !path.exists() {
            error!("Input file not found: {}", path.display());
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                path.to_string_lossy(),
            ));
        }

        let file = fs::File::open(path)?;
        Ok(Input::File(file))
    }
}

impl io::Read for Input {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        match self {
            Input::Stdin(stdin) => stdin.read(buf),
            Input::File(file) => file.read(buf),
        }
    }
}

/// Allow writing to either a file or standard output.
pub enum Output {
    Stdout(io::Stdout),
    File(fs::File),
}

impl Output {
    /// Create an Output from a provided path. If the path is '-'.
    /// then the Output will write to standard output.
    pub fn from_path<T>(path: T) -> Result<Self, io::Error>
    where
        T: AsRef<Path>,
    {
        let path = path.as_ref();

        if path == OsStr::new("-") {
            return Ok(Output::Stdout(io::stdout()));
        }

        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir()?.join(path)
        };

        if !absolute.parent().map(Path::exists).unwrap_or(false) {
            error!(
                "Output file parent directory does not exist: {}",
                absolute.display()
            );

            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                absolute.to_string_lossy(),
            ));
        }

        let file = fs::File::create(absolute)?;
        Ok(Output::File(file))
    }
}

impl io::Write for Output {
    fn write(&mut self, buf: &[u8]) -> Result<usize, io::Error> {
        match self {
            Output::Stdout(stdout) => stdout.write(buf),
            Output::File(file) => file.write(buf),
        }
    }

    fn flush(&mut self) -> Result<(), io::Error> {
        match self {
            Output::Stdout(stdout) => stdout.flush(),
            Output::File(file) => file.flush(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stdinout() {
        assert!(matches!(Input::from_path("-").unwrap(), Input::Stdin(_)));
        assert!(matches!(Output::from_path("-").unwrap(), Output::Stdout(_)));
    }
}
