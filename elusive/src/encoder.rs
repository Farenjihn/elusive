use crate::newc::Archive;

use anyhow::{bail, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use std::str::FromStr;
use thiserror::Error;
use zstd::Encoder as ZstdEncoder;

#[derive(Error, Debug)]
pub enum EncoderError {
    #[error("unknown encoder: {0}")]
    ConversionFailed(String),
}

/// Represents the compression encoder used for an archive
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(PartialEq, Debug)]
pub enum Encoder {
    None,
    Gzip,
    Zstd,
}

impl Encoder {
    /// Encode the provided archive using the specified encoder variant
    pub fn encode_archive<T>(&self, archive: Archive, out: T) -> Result<()>
    where
        T: Write,
    {
        let data = archive.into_bytes()?;
        self.encode(&data, out)
    }

    /// Encode the provided bytes using the specified encoder variant
    pub fn encode<T>(&self, data: &[u8], mut out: T) -> Result<()>
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
                zstdenc.write_all(data)?;
                zstdenc.finish()?;
            }
        }

        Ok(())
    }
}

impl FromStr for Encoder {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Encoder::None),
            "gzip" => Ok(Encoder::Gzip),
            "zstd" => Ok(Encoder::Zstd),
            other => bail!(EncoderError::ConversionFailed(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::newc::EntryBuilder;
    use std::io;

    fn dummy_archive() -> Archive {
        Archive::new(vec![EntryBuilder::file(
            "/testfile",
            b"datadatadata".to_vec(),
        )
        .build()])
    }

    #[test]
    fn test_fromstr() -> Result<()> {
        assert_eq!(Encoder::from_str("none").unwrap(), Encoder::None);
        assert_eq!(Encoder::from_str("gzip").unwrap(), Encoder::Gzip);
        assert_eq!(Encoder::from_str("zstd").unwrap(), Encoder::Zstd);

        assert!(Encoder::from_str("someotherencoder").is_err());

        Ok(())
    }

    #[test]
    fn test_encode() -> Result<()> {
        let sink = io::sink();

        let archive = dummy_archive();
        Encoder::None.encode_archive(archive, sink)?;

        Ok(())
    }

    #[test]
    fn test_encode_ext() -> Result<()> {
        let archive = dummy_archive();
        let data = archive.into_bytes()?;

        let mut none = Vec::new();
        let mut gzip = Vec::new();
        let mut zstd = Vec::new();

        Encoder::None.encode(&data, &mut none)?;
        Encoder::Gzip.encode(&data, &mut gzip)?;
        Encoder::Zstd.encode(&data, &mut zstd)?;

        // gzip should always compress better
        assert!(none.len() > gzip.len());

        // zstd should always compress better
        assert!(none.len() > zstd.len());

        Ok(())
    }
}
