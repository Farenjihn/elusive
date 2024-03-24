//! Convenience types for handling cpio archive compression.

use flate2::write::GzEncoder;
use flate2::Compression;
use std::io;
use std::io::Write;
use std::str::FromStr;
use zstd::Encoder as ZstdEncoder;

/// Custom error type for archive compression handling.
#[derive(thiserror::Error, Debug)]
pub enum EncoderError {
    #[error("i/o error: {0}")]
    InputOutput(io::Error),
    #[error("unknown encoder: {0}")]
    UnknownEncoder(String),
}

impl From<io::Error> for EncoderError {
    fn from(err: io::Error) -> Self {
        Self::InputOutput(err)
    }
}

/// Represents the compression encoder used for an archive.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(PartialEq, Clone, Debug)]
pub enum Encoder {
    None,
    Gzip,
    Zstd,
}

impl Encoder {
    /// Encode the provided bytes using the specified encoder variant.
    pub fn encode<T>(&self, data: &[u8], mut out: T) -> Result<(), EncoderError>
    where
        T: Write,
    {
        match self {
            Encoder::None => {
                out.write_all(data)?;
            }
            Encoder::Gzip => {
                let mut gzenc = GzEncoder::new(&mut out, Compression::default());
                gzenc.write_all(data)?;
            }
            Encoder::Zstd => {
                let mut zstdenc = ZstdEncoder::new(&mut out, 3)?;

                let nproc = num_cpus::get() as u32;
                zstdenc.multithread(nproc)?;

                zstdenc.write_all(data)?;
                zstdenc.finish()?;
            }
        }

        Ok(())
    }
}

impl FromStr for Encoder {
    type Err = EncoderError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Encoder::None),
            "gzip" => Ok(Encoder::Gzip),
            "zstd" => Ok(Encoder::Zstd),
            other => Err(EncoderError::UnknownEncoder(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::newc::Archive;
    use crate::vfs::Entry;
    use std::path::PathBuf;

    fn dummy_archive() -> Archive {
        Archive::from([(PathBuf::from("/test"), Entry::file(b"data".to_vec()))])
    }

    #[test]
    fn test_fromstr() {
        assert_eq!(Encoder::from_str("none").unwrap(), Encoder::None);
        assert_eq!(Encoder::from_str("gzip").unwrap(), Encoder::Gzip);
        assert_eq!(Encoder::from_str("zstd").unwrap(), Encoder::Zstd);

        assert!(Encoder::from_str("someotherencoder").is_err());
    }

    #[test]
    fn test_encode_ext() {
        let archive = dummy_archive();
        let data = archive.serialize().unwrap();

        let mut buf_none = Vec::new();
        let mut buf_gzip = Vec::new();
        let mut buf_zstd = Vec::new();

        Encoder::None.encode(&data, &mut buf_none).unwrap();
        Encoder::Gzip.encode(&data, &mut buf_gzip).unwrap();
        Encoder::Zstd.encode(&data, &mut buf_zstd).unwrap();

        // gzip should always compress better
        assert!(buf_none.len() > buf_gzip.len());

        // zstd should always compress better
        assert!(buf_none.len() > buf_zstd.len());
    }
}
