//! The tagged-chunk container shared by every `.WAD` (per-realm world data,
//! player/monster stats, shop) and `.ROM` (text) file.
//!
//! Header `{u32 directory offset, u32 chunk count}`; the directory is
//! `count` 16-byte entries `{tag[4], u32 offset, u32 count, u32 count}`.
//! Tags are stored byte-reversed (the game builds them as big-endian u32
//! constants and the files are little-endian): `YMNE` on disk is `ENMY`.
//! The game looks chunks up by tag (`docs/chunk-files.md`).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChunkError {
    #[error("file is truncated: needed bytes 0x{0:X}..0x{1:X}")]
    Truncated(usize, usize),
}

#[derive(Debug, Clone)]
pub struct Chunk {
    /// Natural spelling, e.g. `ENMY`.
    pub tag: String,
    pub offset: u32,
    /// Number of records in the chunk.
    pub count: u32,
}

#[derive(Debug, Clone)]
pub struct ChunkFile<'a> {
    data: &'a [u8],
    pub chunks: Vec<Chunk>,
    directory: usize,
}

impl<'a> ChunkFile<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, ChunkError> {
        let h = slice(data, 0, 8)?;
        let directory = le_u32(h, 0) as usize;
        let n = le_u32(h, 4) as usize;
        let dir = slice(data, directory, n * 16)?;
        let chunks = dir
            .chunks(16)
            .map(|e| Chunk {
                tag: e[..4].iter().rev().map(|&b| b as char).collect(),
                offset: le_u32(e, 4),
                count: le_u32(e, 8),
            })
            .collect::<Vec<_>>();
        for c in &chunks {
            if c.offset as usize > directory {
                return Err(ChunkError::Truncated(c.offset as usize, directory));
            }
        }
        Ok(Self { data, chunks, directory })
    }

    pub fn get(&self, tag: &str) -> Option<&Chunk> {
        self.chunks.iter().find(|c| c.tag == tag)
    }

    /// A chunk's bytes: from its offset up to the next chunk (or the
    /// directory), since the directory doesn't store sizes.
    pub fn bytes(&self, tag: &str) -> Option<&'a [u8]> {
        let c = self.get(tag)?;
        let end = self
            .chunks
            .iter()
            .map(|o| o.offset as usize)
            .filter(|&o| o > c.offset as usize)
            .min()
            .unwrap_or(self.directory);
        self.data.get(c.offset as usize..end)
    }

    /// A chunk's records when their size is known.
    pub fn records(&self, tag: &str, size: usize) -> Option<std::slice::Chunks<'a, u8>> {
        let c = self.get(tag)?;
        let bytes = self.data.get(c.offset as usize..c.offset as usize + c.count as usize * size)?;
        Some(bytes.chunks(size))
    }
}

fn slice(data: &[u8], at: usize, len: usize) -> Result<&[u8], ChunkError> {
    data.get(at..at + len).ok_or(ChunkError::Truncated(at, at + len))
}

pub(crate) fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_reversed_tags_and_extents() {
        let mut f = vec![0u8; 8];
        f.extend_from_slice(b"AAAABBBBBB");
        let dir = f.len() as u32;
        for (tag, off, n) in [(b"YMNE", 8u32, 1u32), (b"LVEL", 12, 2)] {
            f.extend_from_slice(tag);
            f.extend_from_slice(&off.to_le_bytes());
            f.extend_from_slice(&n.to_le_bytes());
            f.extend_from_slice(&n.to_le_bytes());
        }
        f[0..4].copy_from_slice(&dir.to_le_bytes());
        f[4..8].copy_from_slice(&2u32.to_le_bytes());
        let c = ChunkFile::parse(&f).unwrap();
        assert_eq!(c.chunks[0].tag, "ENMY");
        assert_eq!(c.bytes("ENMY").unwrap(), b"AAAA");
        assert_eq!(c.bytes("LEVL").unwrap(), b"BBBBBB");
        assert_eq!(c.records("LEVL", 3).unwrap().count(), 2);
        assert!(c.get("NOPE").is_none());
    }

    /// Every .WAD and .ROM on the disc is a valid chunk file.
    #[test]
    fn every_real_wad_and_rom_parses() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let mut files = 0;
        let mut tags = std::collections::BTreeSet::new();
        fn walk(dir: &std::path::Path, f: &mut dyn FnMut(&std::path::Path)) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, f);
                } else {
                    f(&p);
                }
            }
        }
        walk(std::path::Path::new(&root), &mut |p| {
            let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
            if ext != "WAD" && ext != "ROM" || p.file_name().is_some_and(|n| n == "AUDATPS2.ROM") {
                return;
            }
            let data = std::fs::read(p).unwrap();
            let c = ChunkFile::parse(&data).unwrap_or_else(|e| panic!("{p:?}: {e}"));
            for ch in &c.chunks {
                assert!(ch.tag.chars().all(|x| x.is_ascii_uppercase()), "{p:?}: tag {:?}", ch.tag);
                tags.insert(ch.tag.clone());
            }
            files += 1;
        });
        if files > 0 {
            eprintln!("{files} chunk files; tags: {tags:?}");
            assert!(files > 30);
        }
    }
}
