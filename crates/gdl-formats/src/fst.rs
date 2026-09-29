//! GameCube disc filesystem table (FST) and a read-only view of the files it
//! describes, so the runtime can load assets straight out of the user's disc
//! image without an extracted copy.
//!
//! Layout (public GC-Forever / WiiBrew documentation, confirmed against this
//! disc): `fst_offset`/`fst_size` from boot.bin locate a table of 12-byte
//! big-endian entries followed by a NUL-terminated string table. Entry 0 is
//! the root directory; its third word is the total entry count.
//!
//! | offset | file entry | directory entry |
//! | --- | --- | --- |
//! | 0x0 (u8) | 0 | 1 |
//! | 0x1 (u24) | name offset in string table | name offset |
//! | 0x4 (u32) | data offset on disc | parent entry index |
//! | 0x8 (u32) | data length | index one past the last child |

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use thiserror::Error;

use crate::disc::{DiscError, DiscHeader};

#[derive(Debug, Error)]
pub enum FstError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Disc(#[from] DiscError),
    #[error("FST is malformed: {0}")]
    Malformed(&'static str),
    #[error("no such file on disc: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone, Copy)]
pub struct FileEntry {
    pub offset: u32,
    pub length: u32,
}

/// Every file on the disc, keyed by its full path with `/` separators,
/// lowercased (the disc mixes `LEVELS/levelA1/objects.ngc` style casing and
/// the game builds paths like `"levels/level%s"`).
#[derive(Debug, Clone)]
pub struct Fst {
    files: HashMap<String, FileEntry>,
    paths: Vec<String>,
}

impl Fst {
    pub fn parse(table: &[u8]) -> Result<Self, FstError> {
        if table.len() < 12 || table[0] != 1 {
            return Err(FstError::Malformed("root entry is not a directory"));
        }
        let count = be_u32(table, 8) as usize;
        let strings_start = count
            .checked_mul(12)
            .filter(|&n| n <= table.len())
            .ok_or(FstError::Malformed("entry count overruns the table"))?;
        let strings = &table[strings_start..];

        let name_at = |entry: usize| -> Result<String, FstError> {
            let off = (be_u32(table, entry * 12) & 0x00FF_FFFF) as usize;
            let rest = strings
                .get(off..)
                .ok_or(FstError::Malformed("name offset past string table"))?;
            let end = rest
                .iter()
                .position(|&b| b == 0)
                .ok_or(FstError::Malformed("unterminated name"))?;
            Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
        };

        let mut files = HashMap::new();
        let mut paths = Vec::new();
        // Stack of (directory path, index one past its last child).
        let mut dirs: Vec<(String, usize)> = vec![(String::new(), count)];

        for i in 1..count {
            while dirs.last().is_some_and(|&(_, end)| i >= end) {
                dirs.pop();
            }
            let parent = dirs
                .last()
                .map(|(p, _)| p.clone())
                .ok_or(FstError::Malformed("entry outside the root directory"))?;
            let name = name_at(i)?;
            let path = if parent.is_empty() {
                name
            } else {
                format!("{parent}/{name}")
            };

            let word1 = be_u32(table, i * 12 + 4);
            let word2 = be_u32(table, i * 12 + 8);
            if table[i * 12] == 1 {
                dirs.push((path, word2 as usize));
            } else {
                let key = path.to_ascii_lowercase();
                files.insert(key, FileEntry { offset: word1, length: word2 });
                paths.push(path);
            }
        }

        Ok(Self { files, paths })
    }

    /// Case-insensitive lookup of a full `/`-separated path.
    pub fn get(&self, path: &str) -> Option<FileEntry> {
        self.files.get(&path.to_ascii_lowercase()).copied()
    }

    /// Every file path on the disc, original casing, in FST order.
    pub fn paths(&self) -> &[String] {
        &self.paths
    }
}

/// An open disc image: boot header, filesystem table, and a reader.
pub struct Disc {
    pub header: DiscHeader,
    pub fst: Fst,
    reader: BufReader<File>,
}

impl Disc {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FstError> {
        let mut reader = BufReader::new(File::open(path)?);
        let header = DiscHeader::read_from(&mut reader)?;
        reader.seek(SeekFrom::Start(header.fst_offset as u64))?;
        let mut table = vec![0u8; header.fst_size as usize];
        reader.read_exact(&mut table)?;
        let fst = Fst::parse(&table)?;
        Ok(Self { header, fst, reader })
    }

    /// Reads a whole file off the disc by its path (case-insensitive).
    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, FstError> {
        let entry = self
            .fst
            .get(path)
            .ok_or_else(|| FstError::NotFound(path.to_string()))?;
        self.reader.seek(SeekFrom::Start(entry.offset as u64))?;
        let mut data = vec![0u8; entry.length as usize];
        self.reader.read_exact(&mut data)?;
        Ok(data)
    }
}

fn be_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(is_dir: bool, name_off: u32, a: u32, b: u32) -> [u8; 12] {
        let mut e = [0u8; 12];
        e[0..4].copy_from_slice(&(name_off | if is_dir { 0x0100_0000 } else { 0 }).to_be_bytes());
        e[4..8].copy_from_slice(&a.to_be_bytes());
        e[8..12].copy_from_slice(&b.to_be_bytes());
        e
    }

    #[test]
    fn walks_nested_directories() {
        // root, DIR "A" (children 2..4), FILE "A/x", FILE "A/y", FILE "z"
        let names = b"A\0x\0y\0z\0";
        let mut t = Vec::new();
        t.extend(entry(true, 0, 0, 5));
        t.extend(entry(true, 0, 0, 4));
        t.extend(entry(false, 2, 0x100, 10));
        t.extend(entry(false, 4, 0x200, 20));
        t.extend(entry(false, 6, 0x300, 30));
        t.extend_from_slice(names);

        let fst = Fst::parse(&t).unwrap();
        assert_eq!(fst.paths(), ["A/x", "A/y", "z"]);
        assert_eq!(fst.get("a/Y").unwrap().offset, 0x200);
        assert_eq!(fst.get("Z").unwrap().length, 30);
        assert!(fst.get("A").is_none());
    }

    #[test]
    fn rejects_non_directory_root() {
        assert!(matches!(Fst::parse(&[0u8; 12]), Err(FstError::Malformed(_))));
    }

    /// Reads every file under the user's own disc image and checks it against
    /// the user's own extracted copy byte-for-byte. Skips if either is absent.
    #[test]
    fn disc_files_match_extracted_copy() {
        let iso = std::env::var("GAUNTLET_DISC").unwrap_or_else(|_| {
            "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet - Dark Legacy (USA).iso".into()
        });
        // Same convention as the model tests: the extracted disc's top-level
        // `Gauntlet/` folder.
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        if !Path::new(&iso).is_file() || !Path::new(&root).is_dir() {
            eprintln!("skipping: disc image or extracted tree not present");
            return;
        }

        let mut disc = Disc::open(&iso).unwrap();
        let paths = disc.fst.paths().to_vec();
        assert!(paths.len() > 2000, "only {} files on disc", paths.len());
        let mut compared = 0;
        for path in &paths {
            let Some(relative) = path.strip_prefix("Gauntlet/") else { continue };
            let on_disk = Path::new(&root).join(relative);
            if !on_disk.is_file() {
                continue;
            }
            assert_eq!(disc.read(path).unwrap(), std::fs::read(&on_disk).unwrap(), "{path}");
            compared += 1;
        }
        assert!(compared > 2000, "compared only {compared} files");
        eprintln!("{} files on disc, {compared} matched extracted copies", paths.len());
    }
}
