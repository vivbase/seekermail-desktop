//! Minimal ZIP packaging for export bundles (T052).
//!
//! A deliberately tiny, dependency-free writer: entries are STORED
//! (no compression — mbox/JSONL text zips poorly anyway and mail bodies are
//! already on local disk), CRC-32 is computed in a first streaming pass and the
//! bytes are copied in a second, so every local header is complete (no data
//! descriptors → maximum reader compatibility). ZIP64 is not implemented;
//! entries or archives ≥ 4 GiB return an error, which the export task surfaces
//! as a normal failure. Keeping this in-tree avoids a new crates.io dependency
//! that the supply-chain gate (dev/05 §4) would have to vet.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::{AppError, AppResult};

const LOCAL_HEADER_SIG: u32 = 0x0403_4b50;
const CENTRAL_HEADER_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
/// "version made by" / "version needed": 2.0 — plain stored entries.
const ZIP_VERSION: u16 = 20;
const U32_MAX: u64 = u32::MAX as u64;

/// CRC-32 (IEEE 802.3, the ZIP polynomial), bytewise table-driven.
fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

struct Entry {
    name: String,
    crc: u32,
    size: u64,
    offset: u64,
}

/// Streaming STORED-only ZIP writer.
pub struct ZipWriter {
    out: File,
    table: [u32; 256],
    entries: Vec<Entry>,
    offset: u64,
}

impl ZipWriter {
    pub fn create(path: &Path) -> AppResult<Self> {
        let out = File::create(path)
            .map_err(|e| AppError::FsPermission(format!("create zip {}: {e}", path.display())))?;
        Ok(Self {
            out,
            table: crc32_table(),
            entries: Vec::new(),
            offset: 0,
        })
    }

    fn crc_of(&self, src: &Path) -> AppResult<(u32, u64)> {
        let file = File::open(src)
            .map_err(|e| AppError::FsPermission(format!("open {}: {e}", src.display())))?;
        let mut reader = BufReader::new(file);
        let mut crc: u32 = 0xFFFF_FFFF;
        let mut size: u64 = 0;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("zip read: {e}")))?;
            if n == 0 {
                break;
            }
            size += n as u64;
            for &b in &buf[..n] {
                crc = self.table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
            }
        }
        Ok((crc ^ 0xFFFF_FFFF, size))
    }

    fn write_all(&mut self, bytes: &[u8]) -> AppResult<()> {
        self.out
            .write_all(bytes)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("zip write: {e}")))?;
        self.offset += bytes.len() as u64;
        Ok(())
    }

    /// Add one on-disk file under `name` (forward-slash relative path).
    pub fn add_file(&mut self, name: &str, src: &Path) -> AppResult<()> {
        let (crc, size) = self.crc_of(src)?;
        if size >= U32_MAX {
            return Err(AppError::Validation(format!(
                "zip entry too large for zip32: {name}"
            )));
        }
        let header_offset = self.offset;
        if header_offset >= U32_MAX {
            return Err(AppError::Validation("zip archive exceeds 4 GiB".into()));
        }

        // Local file header (all sizes known up front — no data descriptor).
        let name_bytes = name.as_bytes();
        let mut header = Vec::with_capacity(30 + name_bytes.len());
        header.extend_from_slice(&LOCAL_HEADER_SIG.to_le_bytes());
        header.extend_from_slice(&ZIP_VERSION.to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes()); // flags
        header.extend_from_slice(&0u16.to_le_bytes()); // method: STORED
        header.extend_from_slice(&0u16.to_le_bytes()); // mod time (epoch ok)
        header.extend_from_slice(&0u16.to_le_bytes()); // mod date
        header.extend_from_slice(&crc.to_le_bytes());
        header.extend_from_slice(&(size as u32).to_le_bytes()); // compressed
        header.extend_from_slice(&(size as u32).to_le_bytes()); // uncompressed
        header.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes()); // extra len
        header.extend_from_slice(name_bytes);
        self.write_all(&header)?;

        // Second pass: copy the bytes.
        let file = File::open(src)
            .map_err(|e| AppError::FsPermission(format!("open {}: {e}", src.display())))?;
        let mut reader = BufReader::new(file);
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("zip read: {e}")))?;
            if n == 0 {
                break;
            }
            self.write_all(&buf[..n])?;
        }

        self.entries.push(Entry {
            name: name.to_string(),
            crc,
            size,
            offset: header_offset,
        });
        Ok(())
    }

    /// Write the central directory + end record and flush.
    pub fn finish(mut self) -> AppResult<()> {
        let cd_start = self.offset;
        let entries = std::mem::take(&mut self.entries);
        for e in &entries {
            let name_bytes = e.name.as_bytes();
            let mut h = Vec::with_capacity(46 + name_bytes.len());
            h.extend_from_slice(&CENTRAL_HEADER_SIG.to_le_bytes());
            h.extend_from_slice(&ZIP_VERSION.to_le_bytes()); // made by
            h.extend_from_slice(&ZIP_VERSION.to_le_bytes()); // needed
            h.extend_from_slice(&0u16.to_le_bytes()); // flags
            h.extend_from_slice(&0u16.to_le_bytes()); // method
            h.extend_from_slice(&0u16.to_le_bytes()); // time
            h.extend_from_slice(&0u16.to_le_bytes()); // date
            h.extend_from_slice(&e.crc.to_le_bytes());
            h.extend_from_slice(&(e.size as u32).to_le_bytes());
            h.extend_from_slice(&(e.size as u32).to_le_bytes());
            h.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes()); // extra
            h.extend_from_slice(&0u16.to_le_bytes()); // comment
            h.extend_from_slice(&0u16.to_le_bytes()); // disk start
            h.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            h.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            h.extend_from_slice(&(e.offset as u32).to_le_bytes());
            h.extend_from_slice(name_bytes);
            self.write_all(&h)?;
        }
        let cd_size = self.offset - cd_start;

        let mut eocd = Vec::with_capacity(22);
        eocd.extend_from_slice(&EOCD_SIG.to_le_bytes());
        eocd.extend_from_slice(&0u16.to_le_bytes()); // disk
        eocd.extend_from_slice(&0u16.to_le_bytes()); // cd disk
        eocd.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        eocd.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        eocd.extend_from_slice(&(cd_size as u32).to_le_bytes());
        eocd.extend_from_slice(&(cd_start as u32).to_le_bytes());
        eocd.extend_from_slice(&0u16.to_le_bytes()); // comment len
        self.write_all(&eocd)?;

        self.out
            .flush()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("zip flush: {e}")))?;
        Ok(())
    }
}

/// Quick structural sanity check used by tests: EOCD signature near the tail.
pub fn looks_like_zip(path: &Path) -> AppResult<bool> {
    let mut f = File::open(path)
        .map_err(|e| AppError::FsPermission(format!("open {}: {e}", path.display())))?;
    let len = f
        .metadata()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("stat: {e}")))?
        .len();
    if len < 22 {
        return Ok(false);
    }
    f.seek(SeekFrom::End(-22))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("seek: {e}")))?;
    let mut sig = [0u8; 4];
    f.read_exact(&mut sig)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("read: {e}")))?;
    Ok(u32::from_le_bytes(sig) == EOCD_SIG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zips_two_files_with_valid_structure() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, b"hello mbox world\n").unwrap();
        std::fs::write(&b, b"{\"k\":1}\n").unwrap();

        let zip_path = dir.path().join("out.zip");
        let mut w = ZipWriter::create(&zip_path).unwrap();
        w.add_file("a.txt", &a).unwrap();
        w.add_file("nested/b.jsonl", &b).unwrap();
        w.finish().unwrap();

        assert!(looks_like_zip(&zip_path).unwrap());
        let bytes = std::fs::read(&zip_path).unwrap();
        // Local header signature at offset 0.
        assert_eq!(&bytes[0..4], &LOCAL_HEADER_SIG.to_le_bytes());
        // Contains both entry names.
        let hay = String::from_utf8_lossy(&bytes);
        assert!(hay.contains("a.txt"));
        assert!(hay.contains("nested/b.jsonl"));
    }

    #[test]
    fn crc32_known_vector() {
        // CRC-32("123456789") = 0xCBF43926
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("v.txt");
        std::fs::write(&p, b"123456789").unwrap();
        let w = ZipWriter::create(&dir.path().join("x.zip")).unwrap();
        let (crc, size) = w.crc_of(&p).unwrap();
        assert_eq!(size, 9);
        assert_eq!(crc, 0xCBF4_3926);
    }
}
