use crate::newc::Archive;

use anyhow::Result;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use zstd::Encoder as ZstdEncoder;

/// Represents the compression encoder used for an archive
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::newc::EntryBuilder;

    fn dummy_archive() -> Archive {
        Archive::new(vec![EntryBuilder::file(
            "/testfile",
            b"datadatadata".to_vec(),
        )
        .build()])
    }

    #[test]
    fn test_encode() -> Result<()> {
        let archive = dummy_archive();
        Encoder::None.encode_archive(archive)?;

        Ok(())
    }

    #[test]
    fn test_encode_ext() -> Result<()> {
        let archive = dummy_archive();
        let data = archive.into_bytes()?;

        let none_enc = Encoder::None;
        let gzip_enc = Encoder::Gzip;
        let zstd_enc = Encoder::Zstd;

        let none = none_enc.encode(&data)?;
        let gzip = gzip_enc.encode(&data)?;
        let zstd = zstd_enc.encode(&data)?;

        // gzip should always compress better
        assert!(none.len() > gzip.len());

        // zstd should always compress better
        assert!(none.len() > zstd.len());

        Ok(())
    }
}
