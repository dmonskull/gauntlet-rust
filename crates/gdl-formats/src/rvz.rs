//! Dolphin's RVZ compressed disc image, read as if it were the plain `.iso`.
//!
//! Format: Dolphin's public `docs/WiaAndRvz.md` (RVZ is WIA with Zstandard,
//! a bigger group entry and "packing" of junk padding); details not spelt out
//! there follow Dolphin's reader (`WIABlob.cpp`, `WIACompression.cpp`,
//! `LaggedFibonacciGenerator.cpp`). Confirmed by reading the user's RVZ and
//! the matching ISO byte-for-byte (see `docs/disc-format.md`). All integers
//! are big-endian.
//!
//! Only GameCube discs are handled: no Wii partitions, so every byte of the
//! disc lives in "raw data" entries, each split into fixed-size groups that
//! are compressed independently. [`RvzReader`] decompresses just the groups a
//! read touches and keeps the last few around.
//!
//! Supported group compression: none and Zstandard (what Dolphin writes by
//! default). bzip2/LZMA/LZMA2 RVZs are refused with [`RvzError::Unsupported`].

use std::io::{self, Read, Seek, SeekFrom};

use ruzstd::decoding::{BlockDecodingStrategy, FrameDecoder};
use thiserror::Error;

pub const RVZ_MAGIC: [u8; 4] = *b"RVZ\x01";
pub const WIA_MAGIC: [u8; 4] = *b"WIA\x01";

/// `wia_file_head_t` (0x48 bytes) + `wia_disc_t` up to `compr_data`.
const FILE_HEAD_SIZE: usize = 0x48;
const DISC_MIN_SIZE: usize = 0xDC - 7;
/// The first bytes of the disc are stored in the header, not in any group.
const DISC_HEAD_SIZE: usize = 0x80;
/// Raw data entries start on this boundary (the first one says 0x80 but
/// really starts at 0), and junk data restarts its generator every block.
const BLOCK_SIZE: u64 = 0x8000;
const RAW_DATA_ENTRY_SIZE: usize = 0x18;
const RVZ_GROUP_ENTRY_SIZE: usize = 0xC;
/// Decompressed groups kept in memory. Reads of one file walk groups in
/// order, so a handful is plenty.
const GROUP_CACHE: usize = 4;

#[derive(Debug, Error)]
pub enum RvzError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("not an RVZ file")]
    NotRvz,
    #[error("{0}")]
    Unsupported(String),
    #[error("RVZ is malformed: {0}")]
    Malformed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Purge,
    Bzip2,
    Lzma,
    Lzma2,
    Zstd,
}

impl Compression {
    fn from_u32(v: u32) -> Option<Self> {
        Some(match v {
            0 => Self::None,
            1 => Self::Purge,
            2 => Self::Bzip2,
            3 => Self::Lzma,
            4 => Self::Lzma2,
            5 => Self::Zstd,
            _ => return None,
        })
    }
}

/// `wia_file_head_t` and `wia_disc_t`, the fields a GameCube reader needs.
#[derive(Debug, Clone)]
pub struct RvzHeader {
    pub version: u32,
    pub version_compatible: u32,
    /// Size of the uncompressed disc (the equivalent `.iso`).
    pub iso_size: u64,
    /// Size of the RVZ file itself.
    pub file_size: u64,
    /// 1 = GameCube, 2 = Wii.
    pub disc_type: u32,
    pub compression: Compression,
    /// Signed for Zstandard (negative levels exist). Informational.
    pub compression_level: i32,
    /// Uncompressed bytes per group (the last group of an entry may be short).
    pub chunk_size: u32,
    /// The disc's first 0x80 bytes.
    pub disc_head: [u8; DISC_HEAD_SIZE],
    pub partition_count: u32,
    pub raw_data_count: u32,
    pub raw_data_offset: u64,
    pub raw_data_size: u32,
    pub group_count: u32,
    pub group_offset: u64,
    pub group_size: u32,
}

impl RvzHeader {
    /// Parses the file head and disc struct from the start of the file.
    pub fn parse(bytes: &[u8]) -> Result<Self, RvzError> {
        if bytes.len() < 4 || bytes[..4] != RVZ_MAGIC {
            return Err(RvzError::NotRvz);
        }
        if bytes.len() < FILE_HEAD_SIZE + DISC_MIN_SIZE {
            return Err(RvzError::Malformed("header truncated".into()));
        }
        let disc_size = be_u32(bytes, 0x0C) as usize;
        if disc_size < DISC_MIN_SIZE {
            return Err(RvzError::Malformed(format!("disc struct is only {disc_size:#x} bytes")));
        }
        let d = &bytes[FILE_HEAD_SIZE..];
        let compression = be_u32(d, 0x04);
        let compression = Compression::from_u32(compression)
            .ok_or_else(|| RvzError::Malformed(format!("unknown compression method {compression}")))?;
        let chunk_size = be_u32(d, 0x0C);
        // RVZ allows 32 KiB..2 MiB powers of two, or multiples of 2 MiB.
        if chunk_size < BLOCK_SIZE as u32
            || !(chunk_size.is_power_of_two() || chunk_size.is_multiple_of(0x20_0000))
        {
            return Err(RvzError::Malformed(format!("chunk size {chunk_size:#x}")));
        }
        Ok(Self {
            version: be_u32(bytes, 0x04),
            version_compatible: be_u32(bytes, 0x08),
            iso_size: be_u64(bytes, 0x24),
            file_size: be_u64(bytes, 0x2C),
            disc_type: be_u32(d, 0x00),
            compression,
            compression_level: be_u32(d, 0x08) as i32,
            chunk_size,
            disc_head: d[0x10..0x90].try_into().unwrap(),
            partition_count: be_u32(d, 0x90),
            raw_data_count: be_u32(d, 0xB4),
            raw_data_offset: be_u64(d, 0xB8),
            raw_data_size: be_u32(d, 0xC0),
            group_count: be_u32(d, 0xC4),
            group_offset: be_u64(d, 0xC8),
            group_size: be_u32(d, 0xD0),
        })
    }
}

/// A `wia_raw_data_t`, with its start already rounded down to a block.
#[derive(Debug, Clone, Copy)]
struct RawData {
    start: u64,
    end: u64,
    first_group: u32,
}

/// An `rvz_group_t`.
#[derive(Debug, Clone, Copy)]
struct Group {
    file_offset: u64,
    /// Stored with the file's compression method (else stored as is).
    compressed: bool,
    /// Bytes in the file; 0 means the whole group is zeroes.
    stored_size: u32,
    /// Size after decompression, before unpacking; 0 means not packed.
    packed_size: u32,
}

struct CachedGroup {
    index: usize,
    data: Vec<u8>,
}

/// Presents an RVZ file as a `Read + Seek` of the uncompressed disc.
pub struct RvzReader<R> {
    file: R,
    pub header: RvzHeader,
    raw_data: Vec<RawData>,
    groups: Vec<Group>,
    pos: u64,
    /// Most recently used first.
    cache: Vec<CachedGroup>,
    zstd: FrameDecoder,
    scratch: Vec<u8>,
    unpacked: Vec<u8>,
}

impl<R: Read + Seek> RvzReader<R> {
    pub fn new(mut file: R) -> Result<Self, RvzError> {
        let mut head = vec![0u8; FILE_HEAD_SIZE + 0xDC];
        file.seek(SeekFrom::Start(0))?;
        let got = read_up_to(&mut file, &mut head)?;
        let header = RvzHeader::parse(&head[..got])?;

        if header.disc_type != 1 || header.partition_count != 0 {
            return Err(RvzError::Unsupported(
                "RVZ of a Wii disc (only GameCube discs are supported)".into(),
            ));
        }
        if !matches!(header.compression, Compression::None | Compression::Zstd) {
            return Err(RvzError::Unsupported(format!(
                "RVZ with {:?} compression (only Zstandard is supported)",
                header.compression
            )));
        }
        let actual_size = file.seek(SeekFrom::End(0))?;
        if actual_size != header.file_size {
            return Err(RvzError::Malformed(format!(
                "file is {actual_size} bytes, header says {} (truncated download?)",
                header.file_size
            )));
        }

        let mut reader = Self {
            file,
            header,
            raw_data: Vec::new(),
            groups: Vec::new(),
            pos: 0,
            cache: Vec::new(),
            zstd: FrameDecoder::new(),
            scratch: Vec::new(),
            unpacked: Vec::new(),
        };
        let h = reader.header.clone();

        let table = reader.read_table(h.raw_data_offset, h.raw_data_size, h.raw_data_count, RAW_DATA_ENTRY_SIZE)?;
        for e in table.as_chunks::<RAW_DATA_ENTRY_SIZE>().0 {
            let offset = be_u64(e, 0);
            let size = be_u64(e, 8);
            if size == 0 {
                continue;
            }
            let start = offset - offset % BLOCK_SIZE;
            let end = offset
                .checked_add(size)
                .filter(|&end| end <= h.iso_size)
                .ok_or_else(|| RvzError::Malformed("raw data past the end of the disc".into()))?;
            let (first_group, group_count) = (be_u32(e, 0x10), be_u32(e, 0x14));
            if u64::from(group_count) != (end - start).div_ceil(u64::from(h.chunk_size))
                || u64::from(first_group) + u64::from(group_count) > u64::from(h.group_count)
            {
                return Err(RvzError::Malformed("raw data group range".into()));
            }
            reader.raw_data.push(RawData { start, end, first_group });
        }
        reader.raw_data.sort_by_key(|r| r.start);

        let table = reader.read_table(h.group_offset, h.group_size, h.group_count, RVZ_GROUP_ENTRY_SIZE)?;
        reader.groups = table
            .as_chunks::<RVZ_GROUP_ENTRY_SIZE>()
            .0
            .iter()
            .map(|g| {
                let size = be_u32(g, 4);
                Group {
                    file_offset: u64::from(be_u32(g, 0)) * 4,
                    compressed: size & 0x8000_0000 != 0,
                    stored_size: size & 0x7FFF_FFFF,
                    packed_size: be_u32(g, 8),
                }
            })
            .collect();
        Ok(reader)
    }

    /// Size of the uncompressed disc.
    pub fn len(&self) -> u64 {
        self.header.iso_size
    }

    pub fn is_empty(&self) -> bool {
        self.header.iso_size == 0
    }

    /// Reads one of the entry tables (compressed as a whole).
    fn read_table(&mut self, offset: u64, stored: u32, count: u32, entry: usize) -> Result<Vec<u8>, RvzError> {
        let want = count as usize * entry;
        if offset.saturating_add(stored.into()) > self.header.file_size {
            return Err(RvzError::Malformed("entry table past the end of the file".into()));
        }
        let mut raw = vec![0u8; stored as usize];
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut raw)?;
        let mut table = match self.header.compression {
            Compression::Zstd => {
                let mut out = Vec::new();
                zstd_frame(&mut self.zstd, &raw, &mut out)?;
                out
            }
            _ => raw,
        };
        if table.len() < want {
            return Err(RvzError::Malformed(format!("entry table is {} bytes, need {want}", table.len())));
        }
        table.truncate(want);
        Ok(table)
    }

    /// Makes group `index_in_raw` of `raw` the front of the cache.
    fn load_group(&mut self, raw: RawData, index_in_raw: u64) -> io::Result<()> {
        let index = raw.first_group as usize + index_in_raw as usize;
        if let Some(hit) = self.cache.iter().position(|c| c.index == index) {
            let entry = self.cache.remove(hit);
            self.cache.insert(0, entry);
            return Ok(());
        }

        let chunk = u64::from(self.header.chunk_size);
        let disc_offset = raw.start + index_in_raw * chunk;
        let len = chunk.min(raw.end - disc_offset) as usize;
        let group = *self.groups.get(index).ok_or_else(|| malformed("group index past the table"))?;

        let mut data = if self.cache.len() >= GROUP_CACHE {
            self.cache.pop().unwrap().data
        } else {
            Vec::new()
        };
        data.clear();

        if group.stored_size == 0 {
            data.resize(len, 0);
        } else {
            if group.file_offset + u64::from(group.stored_size) > self.header.file_size {
                return Err(malformed(format!("group {index} lies past the end of the file")));
            }
            self.scratch.resize(group.stored_size as usize, 0);
            self.file.seek(SeekFrom::Start(group.file_offset))?;
            self.file.read_exact(&mut self.scratch)?;

            let packed = group.packed_size != 0;
            let expected = if packed { group.packed_size as usize } else { len };
            let target = if packed { &mut self.unpacked } else { &mut data };
            target.clear();
            if group.compressed && self.header.compression == Compression::Zstd {
                target.reserve(expected.min(2 * chunk as usize));
                zstd_frame(&mut self.zstd, &self.scratch, target).map_err(into_io)?;
            } else {
                target.extend_from_slice(&self.scratch[..expected.min(self.scratch.len())]);
            }
            if target.len() != expected {
                return Err(malformed(format!(
                    "group {index} decompressed to {} bytes, expected {expected}",
                    target.len()
                )));
            }
            if packed {
                data.resize(len, 0);
                unpack(&self.unpacked, &mut data, disc_offset)?;
            }
        }

        self.cache.insert(0, CachedGroup { index, data });
        Ok(())
    }
}

impl<R: Read + Seek> Read for RvzReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let pos = self.pos;
        if buf.is_empty() || pos >= self.header.iso_size {
            return Ok(0);
        }
        let n = if pos < DISC_HEAD_SIZE as u64 {
            let head = &self.header.disc_head[pos as usize..];
            let n = head.len().min(buf.len());
            buf[..n].copy_from_slice(&head[..n]);
            n
        } else {
            let raw = *self
                .raw_data
                .iter()
                .find(|r| r.start <= pos && pos < r.end)
                .ok_or_else(|| malformed(format!("disc offset {pos:#x} is not stored in the RVZ")))?;
            let chunk = u64::from(self.header.chunk_size);
            let index_in_raw = (pos - raw.start) / chunk;
            self.load_group(raw, index_in_raw)?;
            let data = &self.cache[0].data;
            let at = (pos - raw.start - index_in_raw * chunk) as usize;
            let n = (data.len() - at).min(buf.len());
            buf[..n].copy_from_slice(&data[at..at + n]);
            n
        };
        self.pos += n as u64;
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for RvzReader<R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.header.iso_size.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = target.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;
        Ok(self.pos)
    }
}

/// Decodes exactly one Zstandard frame from `input` (anything after it is
/// ignored), appending to `out`.
fn zstd_frame(decoder: &mut FrameDecoder, input: &[u8], out: &mut Vec<u8>) -> Result<(), RvzError> {
    let bad = |e: &dyn std::fmt::Display| RvzError::Malformed(format!("zstd: {e}"));
    let mut src = input;
    decoder.reset(&mut src).map_err(|e| bad(&e))?;
    decoder
        .decode_blocks(&mut src, BlockDecodingStrategy::All)
        .map_err(|e| bad(&e))?;
    if !decoder.is_finished() {
        return Err(RvzError::Malformed("zstd frame truncated".into()));
    }
    decoder.collect_to_writer(out).map_err(|e| bad(&e))?;
    Ok(())
}

/// Decodes RVZ packing: a sequence of `u32 size` runs, each either `size`
/// literal bytes or (top bit set) a 68-byte junk seed that regenerates
/// `size & 0x7FFFFFFF` bytes. `disc_offset` is where `out` starts on the disc.
pub fn unpack(packed: &[u8], out: &mut [u8], disc_offset: u64) -> io::Result<()> {
    let (mut p, mut o) = (0usize, 0usize);
    while p < packed.len() {
        let size = be_u32(packed.get(p..p + 4).ok_or_else(|| malformed("packed run header"))?, 0);
        p += 4;
        let n = (size & 0x7FFF_FFFF) as usize;
        let dst = out.get_mut(o..o + n).ok_or_else(|| malformed("packed data overruns the group"))?;
        if size & 0x8000_0000 != 0 {
            let seed = packed.get(p..p + SEED_BYTES).ok_or_else(|| malformed("packed junk seed"))?;
            p += SEED_BYTES;
            let mut junk = JunkGenerator::new(seed.try_into().unwrap());
            junk.skip(((disc_offset + o as u64) % BLOCK_SIZE) as usize);
            junk.fill(dst);
        } else {
            let src = packed.get(p..p + n).ok_or_else(|| malformed("packed literal run"))?;
            dst.copy_from_slice(src);
            p += n;
        }
        o += n;
    }
    if o != out.len() {
        return Err(malformed(format!("packed data covers {o} of {} bytes", out.len())));
    }
    Ok(())
}

pub const SEED_BYTES: usize = 17 * 4;
const LFG_K: usize = 521;
const LFG_J: usize = 32;

/// The lagged Fibonacci generator (xor, j = 32, k = 521) that produced the
/// padding between files on GameCube discs, seeded from 17 big-endian words.
pub struct JunkGenerator {
    /// Words already rearranged into output byte order (see `new`).
    state: [u32; LFG_K],
    bytes: [u8; LFG_K * 4],
    pos: usize,
}

impl JunkGenerator {
    pub fn new(seed: &[u8; SEED_BYTES]) -> Self {
        let mut s = [0u32; LFG_K];
        for (w, b) in s.iter_mut().zip(seed.as_chunks::<4>().0) {
            *w = u32::from_be_bytes(*b);
        }
        for i in 17..LFG_K {
            s[i] = (s[i - 17] << 23) ^ (s[i - 16] >> 9) ^ s[i - 1];
        }
        // Output bytes are `x >> 24, x >> 18, x >> 8, x` (18, not 16). That
        // selection is a per-bit map, so it commutes with the xor-only
        // `advance` and can be applied once up front.
        for x in &mut s {
            *x = (*x & 0xFF00_FFFF) | ((*x >> 2) & 0x00FF_0000);
        }
        let mut g = Self { state: s, bytes: [0; LFG_K * 4], pos: 0 };
        for _ in 0..4 {
            g.advance();
        }
        g.refresh();
        g
    }

    fn advance(&mut self) {
        let s = &mut self.state;
        for i in 0..LFG_J {
            s[i] ^= s[i + LFG_K - LFG_J];
        }
        for i in LFG_J..LFG_K {
            s[i] ^= s[i - LFG_J];
        }
    }

    fn refresh(&mut self) {
        for (b, w) in self.bytes.as_chunks_mut::<4>().0.iter_mut().zip(&self.state) {
            *b = w.to_be_bytes();
        }
    }

    /// Discards `n` bytes of output.
    pub fn skip(&mut self, n: usize) {
        self.pos += n;
        if self.pos >= self.bytes.len() {
            while self.pos >= self.bytes.len() {
                self.advance();
                self.pos -= self.bytes.len();
            }
            self.refresh();
        }
    }

    pub fn fill(&mut self, mut out: &mut [u8]) {
        while !out.is_empty() {
            let n = out.len().min(self.bytes.len() - self.pos);
            out[..n].copy_from_slice(&self.bytes[self.pos..self.pos + n]);
            out = &mut out[n..];
            self.skip(n);
        }
    }
}

fn read_up_to(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..])? {
            0 => break,
            n => got += n,
        }
    }
    Ok(got)
}

fn malformed(why: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("RVZ is malformed: {}", why.into()))
}

fn into_io(e: RvzError) -> io::Error {
    match e {
        RvzError::Io(e) => e,
        other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
    }
}

fn be_u32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

fn be_u64(b: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(b[at..at + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use ruzstd::encoding::{CompressionLevel, compress_to_vec};

    use crate::{Disc, ImageKind};

    const CHUNK: u32 = 0x8000;

    fn test_seed() -> [u8; SEED_BYTES] {
        let mut seed = [0u8; SEED_BYTES];
        for (i, w) in seed.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *w = ((i as u32).wrapping_mul(0x9E37_79B9) ^ 0x0123_4567).to_be_bytes();
        }
        seed
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Known answers from Dolphin's own `LaggedFibonacciGenerator.cpp`,
    /// compiled standalone and fed the same synthetic seed.
    #[test]
    fn junk_matches_dolphins_generator() {
        let mut out = vec![0u8; 0x8000];
        JunkGenerator::new(&test_seed()).fill(&mut out);
        assert_eq!(hex(&out[..16]), "3940c11e1005fc8cc7df64cd9fcea058");
        assert_eq!(hex(&out[2076..2092]), "0e8ee0e3968af2ad2e9a2f39ad681dc0", "across a refill");
        assert_eq!(hex(&out[0x7FF0..]), "67d8329366be23ff7bf967e596bf4877");
        let hash = out.iter().fold(0u32, |h, &b| h.wrapping_mul(31).wrapping_add(b.into()));
        assert_eq!(hash, 0x5f36_5e7d);

        let mut skipped = JunkGenerator::new(&test_seed());
        skipped.skip(0x1234);
        let mut o = [0u8; 16];
        skipped.fill(&mut o);
        assert_eq!(hex(&o), "6eab6b8e57dde428b2942e65f836ac6e");
    }

    #[test]
    fn junk_skip_equals_discarding_output() {
        let mut all = vec![0u8; 3 * LFG_K * 4 + 100];
        JunkGenerator::new(&test_seed()).fill(&mut all);
        for skip in [0, 1, 3, 4, LFG_K * 4 - 1, LFG_K * 4, LFG_K * 4 + 7, 2 * LFG_K * 4 + 50] {
            let mut g = JunkGenerator::new(&test_seed());
            g.skip(skip);
            let mut rest = vec![0u8; all.len() - skip];
            // Odd-sized pieces exercise refills mid-copy.
            for piece in rest.chunks_mut(333) {
                g.fill(piece);
            }
            assert!(rest == all[skip..], "skip {skip}");
        }
    }

    #[test]
    fn unpack_mixes_literal_and_junk_runs() {
        let seed = test_seed();
        let mut packed = Vec::new();
        packed.extend(3u32.to_be_bytes());
        packed.extend(b"abc");
        packed.extend((0x8000_0000u32 | 10).to_be_bytes());
        packed.extend(seed);
        let mut out = [0u8; 13];
        unpack(&packed, &mut out, 0x18000 - 3).unwrap();
        assert_eq!(&out[..3], b"abc");
        let mut junk = [0u8; 10];
        JunkGenerator::new(&seed).fill(&mut junk);
        assert_eq!(out[3..], junk, "junk restarts at the block boundary");

        let mut out = [0u8; 13];
        unpack(&packed, &mut out, 0x18000).unwrap();
        let mut junk = [0u8; 13];
        JunkGenerator::new(&seed).fill(&mut junk);
        assert_eq!(out[3..], junk[3..], "junk offset within its block");

        assert!(unpack(&packed, &mut [0u8; 12], 0).is_err(), "overrun");
        assert!(unpack(&packed, &mut [0u8; 14], 0).is_err(), "short");
    }

    enum Stored {
        Zeros,
        Plain(Vec<u8>),
        Zstd(Vec<u8>),
    }

    /// A small synthetic GameCube RVZ exercising every kind of group, and the
    /// disc bytes it should decode to.
    fn synthetic_rvz() -> (Vec<u8>, Vec<u8>) {
        let iso_size = 4 * CHUNK as usize + 0x1234;
        let mut disc: Vec<u8> = (0..iso_size).map(|i| (i * 7 % 251) as u8).collect();
        let seed = test_seed();
        let junk_at = |disc: &mut Vec<u8>, start: usize, len: usize| {
            let mut g = JunkGenerator::new(&seed);
            g.skip(start % BLOCK_SIZE as usize);
            g.fill(&mut disc[start..start + len]);
        };
        let pack = |disc: &Vec<u8>, runs: &[(usize, usize, bool)]| {
            let mut p = Vec::new();
            for &(start, len, junk) in runs {
                if junk {
                    p.extend((0x8000_0000 | len as u32).to_be_bytes());
                    p.extend(seed);
                } else {
                    p.extend((len as u32).to_be_bytes());
                    p.extend(&disc[start..start + len]);
                }
            }
            p
        };
        let zstd = |data: &[u8]| compress_to_vec(data, CompressionLevel::Fastest);

        // Group 2 is all zeroes; groups 3 and 4 contain junk.
        disc[0x10000..0x18000].fill(0);
        junk_at(&mut disc, 0x18100, 0x7000);
        junk_at(&mut disc, 0x20000, 0x1234);
        let groups = vec![
            (Stored::Zstd(zstd(&disc[0..0x8000])), 0),
            (Stored::Plain(disc[0x8000..0x10000].to_vec()), 0),
            (Stored::Zeros, 0),
            {
                let p = pack(&disc, &[(0x18000, 0x100, false), (0x18100, 0x7000, true), (0x1F100, 0xF00, false)]);
                (Stored::Zstd(zstd(&p)), p.len())
            },
            {
                let p = pack(&disc, &[(0x20000, 0x1234, true)]);
                let len = p.len();
                (Stored::Plain(p), len)
            },
        ];

        let mut raw_table = Vec::new();
        raw_table.extend(0x80u64.to_be_bytes());
        raw_table.extend((iso_size as u64 - 0x80).to_be_bytes());
        raw_table.extend(0u32.to_be_bytes());
        raw_table.extend((groups.len() as u32).to_be_bytes());
        let raw_table = zstd(&raw_table);

        let mut file = vec![0u8; FILE_HEAD_SIZE + 0xDC];
        let raw_off = file.len();
        file.extend(&raw_table);
        let mut group_table = Vec::new();
        for (stored, packed) in &groups {
            while !file.len().is_multiple_of(4) {
                file.push(0);
            }
            let (bytes, flag): (&[u8], u32) = match stored {
                Stored::Zeros => (&[], 0),
                Stored::Plain(b) => (b, 0),
                Stored::Zstd(b) => (b, 0x8000_0000),
            };
            group_table.extend((file.len() as u32 / 4).to_be_bytes());
            group_table.extend((flag | bytes.len() as u32).to_be_bytes());
            group_table.extend((*packed as u32).to_be_bytes());
            file.extend(bytes);
        }
        let group_table = zstd(&group_table);
        let group_off = file.len();
        file.extend(&group_table);

        let file_size = file.len() as u64;
        let h = &mut file[..FILE_HEAD_SIZE + 0xDC];
        h[0..4].copy_from_slice(&RVZ_MAGIC);
        h[4..8].copy_from_slice(&0x0100_0000u32.to_be_bytes());
        h[8..12].copy_from_slice(&0x0003_0000u32.to_be_bytes());
        h[0x0C..0x10].copy_from_slice(&0xDCu32.to_be_bytes());
        h[0x24..0x2C].copy_from_slice(&(iso_size as u64).to_be_bytes());
        h[0x2C..0x34].copy_from_slice(&file_size.to_be_bytes());
        let d = &mut h[FILE_HEAD_SIZE..];
        d[0x00..0x04].copy_from_slice(&1u32.to_be_bytes());
        d[0x04..0x08].copy_from_slice(&5u32.to_be_bytes());
        d[0x08..0x0C].copy_from_slice(&(-3i32).to_be_bytes());
        d[0x0C..0x10].copy_from_slice(&CHUNK.to_be_bytes());
        d[0x10..0x90].copy_from_slice(&disc[..0x80]);
        d[0x94..0x98].copy_from_slice(&0x30u32.to_be_bytes());
        d[0xB4..0xB8].copy_from_slice(&1u32.to_be_bytes());
        d[0xB8..0xC0].copy_from_slice(&(raw_off as u64).to_be_bytes());
        d[0xC0..0xC4].copy_from_slice(&(raw_table.len() as u32).to_be_bytes());
        d[0xC4..0xC8].copy_from_slice(&(groups.len() as u32).to_be_bytes());
        d[0xC8..0xD0].copy_from_slice(&(group_off as u64).to_be_bytes());
        d[0xD0..0xD4].copy_from_slice(&(group_table.len() as u32).to_be_bytes());
        (file, disc)
    }

    #[test]
    fn parses_header_fields() {
        let (file, disc) = synthetic_rvz();
        let h = RvzHeader::parse(&file).unwrap();
        assert_eq!(h.version, 0x0100_0000);
        assert_eq!(h.iso_size, disc.len() as u64);
        assert_eq!(h.file_size, file.len() as u64);
        assert_eq!((h.disc_type, h.compression, h.compression_level), (1, Compression::Zstd, -3));
        assert_eq!(h.chunk_size, CHUNK);
        assert_eq!(h.disc_head[..], disc[..0x80]);
        assert_eq!((h.raw_data_count, h.group_count, h.partition_count), (1, 5, 0));

        assert!(matches!(RvzHeader::parse(b"WIA\x01"), Err(RvzError::NotRvz)));
        assert!(matches!(RvzHeader::parse(&file[..0x60]), Err(RvzError::Malformed(_))));
        let mut bad = file.clone();
        bad[FILE_HEAD_SIZE + 0x0C..FILE_HEAD_SIZE + 0x10].copy_from_slice(&0x9000u32.to_be_bytes());
        assert!(matches!(RvzHeader::parse(&bad), Err(RvzError::Malformed(_))), "odd chunk size");
    }

    #[test]
    fn refuses_unsupported_rvz_variants() {
        let (file, _) = synthetic_rvz();
        let mut lzma = file.clone();
        lzma[FILE_HEAD_SIZE + 4..FILE_HEAD_SIZE + 8].copy_from_slice(&3u32.to_be_bytes());
        assert!(matches!(RvzReader::new(Cursor::new(lzma)), Err(RvzError::Unsupported(_))));
        let mut wii = file.clone();
        wii[FILE_HEAD_SIZE..FILE_HEAD_SIZE + 4].copy_from_slice(&2u32.to_be_bytes());
        assert!(matches!(RvzReader::new(Cursor::new(wii)), Err(RvzError::Unsupported(_))));
        let truncated = file[..file.len() - 1].to_vec();
        assert!(matches!(RvzReader::new(Cursor::new(truncated)), Err(RvzError::Malformed(_))));
    }

    #[test]
    fn decodes_every_group_kind() {
        let (file, disc) = synthetic_rvz();
        let mut r = RvzReader::new(Cursor::new(file)).unwrap();
        assert_eq!(r.len(), disc.len() as u64);

        let mut all = Vec::new();
        r.read_to_end(&mut all).unwrap();
        assert!(all == disc, "whole disc");

        // Reads that straddle the header, group edges and the disc end.
        for (at, len) in [(0x70, 0x20), (0x7FF0, 0x20), (0x100F8, 0x10), (0x180F0, 0x7020), (0x20000, 0x1234), (5, 0x21000)] {
            r.seek(SeekFrom::Start(at)).unwrap();
            let mut buf = vec![0u8; len];
            r.read_exact(&mut buf).unwrap();
            assert!(buf == disc[at as usize..at as usize + len], "{at:#x}+{len:#x}");
        }
        r.seek(SeekFrom::End(-4)).unwrap();
        let mut tail = Vec::new();
        r.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, disc[disc.len() - 4..]);
    }

    /// The user's RVZ and the ISO of the same disc (`GAUNTLET_RVZ`,
    /// `GAUNTLET_DISC`), when both are present.
    fn real_rvz_and_iso() -> Option<(String, String)> {
        let dir = "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet - Dark Legacy (USA)";
        let rvz = std::env::var("GAUNTLET_RVZ").unwrap_or_else(|_| format!("{dir}.rvz"));
        let iso = std::env::var("GAUNTLET_DISC").unwrap_or_else(|_| format!("{dir}.iso"));
        if std::path::Path::new(&rvz).is_file() && std::path::Path::new(&iso).is_file() {
            Some((rvz, iso))
        } else {
            eprintln!("skipping: RVZ or ISO not present");
            None
        }
    }

    /// The user's RVZ must read back exactly as the matching ISO: boot
    /// header, FST, every file, and the junk padding between files.
    #[test]
    fn real_rvz_matches_iso() {
        let Some((rvz, iso)) = real_rvz_and_iso() else { return };
        let started = std::time::Instant::now();
        let mut a = Disc::open(&rvz).unwrap();
        let mut b = Disc::open(&iso).unwrap();
        assert_eq!((a.kind, b.kind), (ImageKind::Rvz, ImageKind::GameCube));
        assert_eq!(a.header.game_id, b.header.game_id);
        assert_eq!(a.header.title, b.header.title);
        assert_eq!(
            (a.header.dol_offset, a.header.fst_offset, a.header.fst_size),
            (b.header.dol_offset, b.header.fst_offset, b.header.fst_size)
        );
        assert_eq!(a.fst.paths(), b.fst.paths());
        let iso_len = std::fs::metadata(&iso).unwrap().len();

        let paths = a.fst.paths().to_vec();
        let mut bytes = 0u64;
        for p in &paths {
            let (x, y) = (a.read(p).unwrap(), b.read(p).unwrap());
            assert!(x == y, "{p} differs");
            bytes += x.len() as u64;
        }
        eprintln!("{} files, {bytes} bytes identical in {:?}", paths.len(), started.elapsed());

        // Everything the FST doesn't cover: system area, the gaps between
        // files (junk padding) and the tail of the disc.
        let mut extents: Vec<(u64, u64)> = paths
            .iter()
            .map(|p| a.fst.get(p).unwrap())
            .map(|e| (e.offset as u64, e.offset as u64 + e.length as u64))
            .collect();
        extents.sort();
        let mut gaps = vec![(0, extents[0].0)];
        let mut end = extents[0].1;
        for &(s, e) in &extents[1..] {
            if s > end {
                gaps.push((end, s));
            }
            end = end.max(e);
        }
        gaps.push((end, iso_len));
        let mut gap_bytes = 0u64;
        for &(s, e) in &gaps {
            // Big gaps (the ~190 MB of junk after the last file) are
            // sampled: both seams, then 64 KiB every 4 MiB.
            let spans = if e - s <= 0x40000 {
                vec![(s, e)]
            } else {
                let mut v = vec![(s, s + 0x20000), (e - 0x20000, e)];
                v.extend((s + 0x20000..e - 0x30000).step_by(0x40_0000).map(|o| (o, o + 0x10000)));
                v
            };
            for (s, e) in spans {
                let len = (e - s) as usize;
                let (x, y) = (a.read_at(s, len).unwrap(), b.read_at(s, len).unwrap());
                assert!(x == y, "gap {s:#x}..{e:#x} differs");
                gap_bytes += e - s;
            }
        }
        assert!(gap_bytes > 2 << 20, "only {gap_bytes} bytes outside files compared");
        eprintln!(
            "{} gaps, {gap_bytes} bytes of system area and junk padding identical; total {:?}",
            gaps.len(),
            started.elapsed()
        );
    }

    /// Every byte of the disc, sequentially (about as quick as the per-file
    /// test: ~8 s for 1.4 GB).
    #[test]
    fn real_rvz_whole_disc_matches_iso() {
        let Some((rvz, iso)) = real_rvz_and_iso() else { return };
        let started = std::time::Instant::now();
        let mut a = RvzReader::new(std::fs::File::open(rvz).unwrap()).unwrap();
        let mut b = std::fs::File::open(iso).unwrap();
        assert_eq!(a.len(), b.metadata().unwrap().len());
        let (mut x, mut y) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
        let mut at = 0u64;
        while at < a.len() {
            let n = (a.len() - at).min(x.len() as u64) as usize;
            a.read_exact(&mut x[..n]).unwrap();
            b.read_exact(&mut y[..n]).unwrap();
            assert!(x[..n] == y[..n], "differs in {at:#x}..+{n:#x}");
            at += n as u64;
        }
        eprintln!("{at} bytes identical in {:?}", started.elapsed());
    }
}
