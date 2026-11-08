//! NCZ/NSZ/XCZ compression format support.
//!
//! NCZ is a compressed NCA. An NSZ is a PFS0 of NCZ files.
//! An XCZ is an XCI with NCZ files in the secure partition.
//!
//! NCZ format:
//! - First 0x4000 bytes: original NCA header + section headers (unmodified)
//! - NCZSECTN magic + section table at 0x4000
//! - Then EITHER:
//!   - Block table (version, block_size, etc.) + compressed blocks  (block-based)
//!   - Raw zstd stream (starts with zstd magic 0xFD2FB528)          (stream-based)

use std::io::{Read, Seek, SeekFrom, Write};

use aes::cipher::KeyInit;
use aes::Aes128;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use indicatif::ProgressBar;

use crate::error::{NscbError, Result};

/// Magic bytes at the start of the NCZ section table.
pub const NCZ_MAGIC: &[u8; 8] = b"NCZSECTN";

/// Magic bytes at the start of a block-based NCZ block table.
pub const NCZBLOCK_MAGIC: &[u8; 8] = b"NCZBLOCK";

/// Zstd frame magic (little-endian).
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// NCZ section entry — describes one encrypted section of the NCA.
#[derive(Debug, Clone)]
pub struct NczSection {
    pub offset: u64,
    pub size: u64,
    pub crypto_type: u64,
    pub _padding: u64,
    pub crypto_key: [u8; 16],
    pub crypto_counter: [u8; 16],
}

impl NczSection {
    pub fn has_valid_range(&self) -> bool {
        self.size > 0 && self.offset >= 0x4000
    }
}

/// NCZ block table — maps compressed blocks to decompressed data.
#[derive(Debug, Clone)]
pub struct NczBlockTable {
    pub version: u8,
    pub block_type: u8,
    pub _unused: u8,
    pub block_size_exponent: u8,
    pub block_count: u32,
    pub decompressed_size: u64,
    /// Compressed sizes for each block.
    pub block_sizes: Vec<u32>,
}

/// Parse NCZ metadata from a reader.
pub struct NczReader {
    /// Sections describing the encrypted layout.
    pub sections: Vec<NczSection>,
    /// Block table for block-based decompression (None for stream-based).
    pub block_table: Option<NczBlockTable>,
    /// Whether this is a stream-based NCZ (raw zstd after sections).
    pub is_stream: bool,
    /// Offset where the compressed data starts (after header + sections + optional block table).
    pub data_start: u64,
    /// The original NCA header bytes (first 0x4000 bytes).
    pub nca_header: Vec<u8>,
}

impl NczReader {
    /// Parse NCZ from a reader at current position.
    pub fn parse<R: Read + Seek>(reader: &mut R) -> Result<Self> {
        let pos = reader.stream_position()?;
        Self::parse_at(reader, pos)
    }

    /// Parse NCZ from a reader at a specific offset.
    pub fn parse_at<R: Read + Seek>(reader: &mut R, offset: u64) -> Result<Self> {
        reader.seek(SeekFrom::Start(offset))?;

        // Read original NCA header (0x4000 bytes)
        let mut nca_header = vec![0u8; 0x4000];
        reader.read_exact(&mut nca_header)?;

        // Check for section magic
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;

        if &magic != NCZ_MAGIC {
            return Err(NscbError::InvalidMagic {
                expected: "NCZSECTN".into(),
                got: String::from_utf8_lossy(&magic).into(),
            });
        }

        let section_count = reader.read_u64::<LittleEndian>()? as usize;

        // Parse sections (64 bytes each)
        let mut sections = Vec::with_capacity(section_count);
        for _ in 0..section_count {
            let sec_offset = reader.read_u64::<LittleEndian>()?;
            let size = reader.read_u64::<LittleEndian>()?;
            let crypto_type = reader.read_u64::<LittleEndian>()?;
            let padding = reader.read_u64::<LittleEndian>()?;

            let mut crypto_key = [0u8; 16];
            reader.read_exact(&mut crypto_key)?;

            let mut crypto_counter = [0u8; 16];
            reader.read_exact(&mut crypto_counter)?;

            sections.push(NczSection {
                offset: sec_offset,
                size,
                crypto_type,
                _padding: padding,
                crypto_key,
                crypto_counter,
            });
        }

        // Check what follows: block table or raw zstd stream?
        let after_sections = reader.stream_position()?;
        let mut peek = [0u8; 4];
        reader.read_exact(&mut peek)?;

        if peek == ZSTD_MAGIC {
            // Stream-based NCZ: raw zstd data follows sections
            Ok(Self {
                sections,
                block_table: None,
                is_stream: true,
                data_start: after_sections,
                nca_header,
            })
        } else {
            // Block-based NCZ: parse block table
            // Seek back and read block table header
            reader.seek(SeekFrom::Start(after_sections))?;
            let block_table = Self::parse_block_table(reader)?;
            let data_start = reader.stream_position()?;

            Ok(Self {
                sections,
                block_table: Some(block_table),
                is_stream: false,
                data_start,
                nca_header,
            })
        }
    }

    fn parse_block_table<R: Read + Seek>(reader: &mut R) -> Result<NczBlockTable> {
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;
        if &magic != NCZBLOCK_MAGIC {
            return Err(NscbError::InvalidMagic {
                expected: "NCZBLOCK".into(),
                got: String::from_utf8_lossy(&magic).into_owned(),
            });
        }

        let version = reader.read_u8()?;
        let block_type = reader.read_u8()?;
        let unused = reader.read_u8()?;
        let block_size_exponent = reader.read_u8()?;
        let block_count = reader.read_u32::<LittleEndian>()?;
        let decompressed_size = reader.read_u64::<LittleEndian>()?;

        let mut block_sizes = Vec::with_capacity(block_count as usize);
        for _ in 0..block_count {
            let bs = reader.read_u32::<LittleEndian>()?;
            block_sizes.push(bs);
        }

        Ok(NczBlockTable {
            version,
            block_type,
            _unused: unused,
            block_size_exponent,
            block_count,
            decompressed_size,
            block_sizes,
        })
    }

    /// Block size in bytes.
    pub fn block_size(&self) -> usize {
        match &self.block_table {
            Some(bt) => 1 << bt.block_size_exponent,
            None => 1 << 15, // 32KB default
        }
    }
}

/// Decompress an NCZ file to NCA.
/// `offset` is where the NCZ data starts in the reader.
impl NczReader {
    /// Size of the decompressed NCA: the end of the last section, at least the header.
    pub fn nca_size(&self, fallback: u64) -> u64 {
        let end = self.sections.iter().map(|s| s.offset + s.size).max().unwrap_or(fallback);
        end.max(self.nca_header.len() as u64)
    }

    /// SHA-256 of the first `n` bytes of the decompressed NCA (as stored in HFS0 headers),
    /// computable before decompressing.
    pub fn nca_prefix_hash(&self, n: usize) -> Result<[u8; 32]> {
        let n = n.min(self.nca_header.len());
        let mut prefix = self.nca_header[..n].to_vec();
        for (start, end, cipher, nonce) in crypt_ranges(&self.sections)? {
            apply_range(&mut prefix, 0, start, end, &cipher, &nonce);
        }
        Ok(crate::crypto::hash::sha256(&prefix))
    }
}

/// AES-CTR ranges to re-apply after decompression (crypto types 3 and 4), matching
/// NSC_BUILDER: types 0 and 1 are left as-is.
fn crypt_ranges(sections: &[NczSection]) -> Result<Vec<(u64, u64, Aes128, [u8; 8])>> {
    let mut ranges = Vec::new();
    for s in sections {
        match s.crypto_type {
            0 | 1 => {}
            3 | 4 => {
                let cipher = Aes128::new_from_slice(&s.crypto_key)
                    .map_err(|e| NscbError::Crypto(format!("AES key init: {e}")))?;
                let mut nonce = [0u8; 8];
                nonce.copy_from_slice(&s.crypto_counter[..8]);
                ranges.push((s.offset, s.offset.saturating_add(s.size), cipher, nonce));
            }
            other => {
                return Err(NscbError::InvalidData(format!(
                    "Unsupported NCZ section crypto type {other}"
                )))
            }
        }
    }
    Ok(ranges)
}

/// Apply the CTR transform for `[start, end)` to the part of `data` (which begins at
/// absolute offset `pos`) that overlaps it.
fn apply_range(data: &mut [u8], pos: u64, start: u64, end: u64, cipher: &Aes128, nonce: &[u8; 8]) {
    let lo = start.max(pos);
    let hi = end.min(pos + data.len() as u64);
    if lo < hi {
        let a = (lo - pos) as usize;
        let b = (hi - pos) as usize;
        aes_ctr_transform_in_place(cipher, nonce, lo, &mut data[a..b]);
    }
}

/// Writer that re-encrypts NCZ sections as decompressed bytes pass through, tracking the
/// absolute NCA offset, and advances an optional progress bar.
struct SectionCryptWriter<'a, W: Write> {
    inner: &'a mut W,
    pos: u64,
    ranges: Vec<(u64, u64, Aes128, [u8; 8])>,
    scratch: Vec<u8>,
    pb: Option<&'a ProgressBar>,
}

impl<W: Write> Write for SectionCryptWriter<'_, W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let end = self.pos + data.len() as u64;
        let overlaps = self.ranges.iter().any(|(s, e, _, _)| *s < end && *e > self.pos);
        if overlaps {
            self.scratch.clear();
            self.scratch.extend_from_slice(data);
            for (start, stop, cipher, nonce) in &self.ranges {
                apply_range(&mut self.scratch, self.pos, *start, *stop, cipher, nonce);
            }
            self.inner.write_all(&self.scratch)?;
        } else {
            self.inner.write_all(data)?;
        }
        self.pos = end;
        if let Some(pb) = self.pb {
            pb.inc(data.len() as u64);
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Decompress an NCZ into `writer` as the original NCA, in a single pass.
pub fn decompress_ncz<R: Read + Seek, W: Write>(
    reader: &mut R,
    writer: &mut W,
    total_nca_size: u64,
    offset: u64,
    compressed_size: u64,
) -> Result<()> {
    decompress_ncz_with_progress(reader, writer, total_nca_size, offset, compressed_size, None).map(|_| ())
}

/// Like [`decompress_ncz`], advancing `pb` by output bytes. Returns the bytes written.
pub fn decompress_ncz_with_progress<R: Read + Seek, W: Write>(
    reader: &mut R,
    writer: &mut W,
    total_nca_size: u64,
    offset: u64,
    compressed_size: u64,
    pb: Option<&ProgressBar>,
) -> Result<u64> {
    let ncz = NczReader::parse_at(reader, offset)?;
    // Sections are re-encrypted on the way out (cryptoType 3/4 AES-CTR), matching
    // NSC_BUILDER's in-place transform of the decompressed payload.
    let mut out = SectionCryptWriter {
        inner: writer,
        pos: 0,
        ranges: crypt_ranges(&ncz.sections)?,
        scratch: Vec::new(),
        pb,
    };

    // Write the original NCA header
    out.write_all(&ncz.nca_header)?;

    // Seek to compressed data
    reader.seek(SeekFrom::Start(ncz.data_start))?;
    let stream_size = compressed_size.saturating_sub(ncz.data_start.saturating_sub(offset));
    let mut limited = reader.take(stream_size);

    let mut written = ncz.nca_header.len() as u64;
    let target = total_nca_size;

    if ncz.is_stream {
        // Stream-based: decompress a single zstd stream
        decompress_stream(&mut limited, &mut out, &mut written, target)?;
    } else if let Some(block_table) = &ncz.block_table {
        // Block-based: decompress individual blocks
        let block_size = 1usize << block_table.block_size_exponent;
        decompress_blocks(
            &mut limited,
            &mut out,
            block_table,
            block_size,
            &mut written,
            target,
        )?;
    } else {
        return Err(NscbError::InvalidData(
            "NCZ has neither block table nor zstd stream".into(),
        ));
    }

    // Pad to target size if needed
    if written < target {
        crate::util::io::write_padding(&mut out, target - written)?;
        written = target;
    }
    out.flush()?;
    Ok(written)
}

fn aes_ctr_transform_in_place(cipher: &Aes128, nonce8: &[u8], file_offset: u64, data: &mut [u8]) {
    crate::crypto::aes_ctr::ctr64be_xor(cipher, nonce8, file_offset, data);
}

/// Decompress a stream-based NCZ (single zstd stream).
fn decompress_stream<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    written: &mut u64,
    target: u64,
) -> Result<()> {
    let mut decoder = zstd::Decoder::new(reader)
        .map_err(|e| NscbError::Crypto(format!("zstd decoder init error: {e}")))?;

    let mut buf = vec![0u8; 1024 * 1024]; // 1MB read buffer
    loop {
        crate::util::report::check_cancelled()?;
        if *written >= target {
            break;
        }

        let to_read = buf.len().min((target - *written) as usize);
        let n = decoder
            .read(&mut buf[..to_read])
            .map_err(|e| NscbError::Crypto(format!("zstd decompress error: {e}")))?;

        if n == 0 {
            break;
        }

        writer.write_all(&buf[..n])?;
        *written += n as u64;
    }

    Ok(())
}

/// Decompress block-based NCZ.
fn decompress_blocks<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    block_table: &NczBlockTable,
    block_size: usize,
    written: &mut u64,
    target: u64,
) -> Result<()> {
    for &compressed_size in &block_table.block_sizes {
        crate::util::report::check_cancelled()?;
        if *written >= target {
            break;
        }

        let mut compressed = vec![0u8; compressed_size as usize];
        reader.read_exact(&mut compressed)?;

        // Check if block is stored uncompressed (compressed_size == block_size)
        let decompressed = if compressed_size as usize == block_size {
            compressed
        } else {
            zstd::decode_all(&compressed[..])
                .map_err(|e| NscbError::Crypto(format!("zstd decompress error: {e}")))?
        };

        let to_write = decompressed.len().min((target - *written) as usize);
        writer.write_all(&decompressed[..to_write])?;
        *written += to_write as u64;
    }

    Ok(())
}

/// Compress an NCA to NCZ format (block-based).
pub fn compress_nca<R: Read + Seek, W: Write + Seek>(
    reader: &mut R,
    writer: &mut W,
    nca_size: u64,
    sections: &[NczSection],
    compression_level: i32,
    pb: Option<&ProgressBar>,
) -> Result<()> {
    let nca_base = reader.stream_position()?;

    // Copy original NCA header (0x4000 bytes)
    let mut nca_header = vec![0u8; 0x4000];
    reader.read_exact(&mut nca_header)?;
    writer.write_all(&nca_header)?;

    // Write section magic + section table, then a solid zstd stream.
    writer.write_all(NCZ_MAGIC)?;
    let mut out_sections: Vec<NczSection> = sections
        .iter()
        .filter(|s| s.has_valid_range())
        .cloned()
        .collect();
    if out_sections.is_empty() {
        let data_to_compress = nca_size.saturating_sub(0x4000);
        if data_to_compress > 0 {
            out_sections.push(NczSection {
                offset: 0x4000,
                size: data_to_compress,
                crypto_type: 1,
                _padding: 0,
                crypto_key: [0u8; 16],
                crypto_counter: [0u8; 16],
            });
        }
    } else {
        out_sections.sort_by_key(|s| s.offset);
    }

    writer.write_u64::<LittleEndian>(out_sections.len() as u64)?;
    for s in &out_sections {
        writer.write_u64::<LittleEndian>(s.offset)?;
        writer.write_u64::<LittleEndian>(s.size)?;
        writer.write_u64::<LittleEndian>(s.crypto_type)?;
        writer.write_u64::<LittleEndian>(0)?;
        writer.write_all(&s.crypto_key)?;
        writer.write_all(&s.crypto_counter)?;
    }

    if nca_size > 0x4000 {
        let mut enc = zstd::stream::write::Encoder::new(writer, compression_level)
            .map_err(|e| NscbError::Crypto(format!("zstd encoder init error: {e}")))?;
        // Compress on all cores. The output is still a single standard zstd frame.
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get() as u32);
        if workers > 1 {
            enc.multithread(workers)
                .map_err(|e| NscbError::Crypto(format!("zstd multithread setup error: {e}")))?;
        }
        let mut pos = 0x4000u64;
        for s in &out_sections {
            if s.offset > pos {
                stream_range(reader, &mut enc, nca_base, pos, s.offset - pos, None, pb)?;
            }
            stream_range(reader, &mut enc, nca_base, s.offset, s.size, Some(s), pb)?;
            pos = s.offset.saturating_add(s.size);
        }
        if pos < nca_size {
            stream_range(reader, &mut enc, nca_base, pos, nca_size - pos, None, pb)?;
        }
        enc.finish()
            .map_err(|e| NscbError::Crypto(format!("zstd stream finalize error: {e}")))?;
    }

    Ok(())
}

fn stream_range<R: Read + Seek, W: Write>(
    reader: &mut R,
    writer: &mut W,
    nca_base: u64,
    rel_offset: u64,
    size: u64,
    section: Option<&NczSection>,
    pb: Option<&ProgressBar>,
) -> Result<()> {
    if size == 0 {
        return Ok(());
    }
    reader.seek(SeekFrom::Start(nca_base + rel_offset))?;
    let mut remaining = size;
    let mut abs = rel_offset;
    let mut buf = vec![0u8; 1024 * 1024];
    while remaining > 0 {
        crate::util::report::check_cancelled()?;
        let take = remaining.min(buf.len() as u64) as usize;
        reader.read_exact(&mut buf[..take])?;
        if let Some(s) = section {
            if s.crypto_type == 3 || s.crypto_type == 4 {
                let cipher = Aes128::new_from_slice(&s.crypto_key)
                    .map_err(|e| NscbError::Crypto(format!("AES key init: {e}")))?;
                aes_ctr_transform_in_place(&cipher, &s.crypto_counter[..8], abs, &mut buf[..take]);
            }
        }
        writer.write_all(&buf[..take])?;
        if let Some(pb) = pb {
            pb.inc(take as u64);
        }
        remaining -= take as u64;
        abs += take as u64;
    }
    Ok(())
}
