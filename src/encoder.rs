use crate::newc::Archive;

use anyhow::Result;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use zstd::stream::write::Encoder as ZstdEncoder;

/// Represents the compression encoder used for an archive
pub enum Encoder {
    None,
    Gzip,
    Zstd,
}

impl Encoder {
    /// Encode the provided archive using the specified encoder variant
    pub fn encode(&self, archive: Archive) -> Result<Vec<u8>> {
        let data = archive.into_bytes()?;
        let mut buf = Vec::new();

        match self {
            Encoder::None => return Ok(data),
            Encoder::Gzip => {
                let mut gzenc = GzEncoder::new(&mut buf, Compression::default());
                gzenc.write_all(&data)?;
            }
            Encoder::Zstd => {
                let mut zstdenc = ZstdEncoder::new(&mut buf, 3)?;
                zstdenc.write_all(&data)?;
                zstdenc.finish()?;
            }
        }

        Ok(buf)
    }
}
