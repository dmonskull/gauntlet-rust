//! `objects.ngc` model file header.
//!
//! Reverse engineered from the fixup routine at `0x800b7534` in `main.dol`
//! (function `FUN_800b7534`, reached from the string reference
//! `"model %d %s has a bad version (%08x) (want %08x)"`). See
//! `docs/objects-ngc-format.md` for the full writeup — this only covers the
//! header; per-entry array layouts are not implemented yet because they
//! aren't confirmed against the binary.

use std::io::{self, Read, Seek, SeekFrom};

use thiserror::Error;

/// Version values observed on real disc data. `0x800b7534` doesn't compare
/// against a single hardcoded constant in a way we can prove is exhaustive,
/// so treat this as "known good", not "the only valid value".
pub const KNOWN_VERSIONS: [u32; 2] = [0xF00B000D, 0xF00B000C];

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("unrecognized version 0x{0:08X} (known: {KNOWN_VERSIONS:X?})")]
    UnknownVersion(u32),
}

/// The confirmed portion of an `objects.ngc` header (offsets `0x40..0x80`).
/// The file is little-endian on disk in its entirety.
#[derive(Debug, Clone)]
pub struct ModelHeader {
    pub version: u32,
    pub num_objects: u32,
    pub num_b: u32,
    pub num_objects_dup: u32,
    pub num_d: u32,
    pub objects_offset: u32,
    pub objects_b_offset: u32,
    pub bounds_offset: u32,
    pub d_offset: u32,
    pub unnamed_0x64: u32,
    pub unnamed_0x68: u32,
    pub unnamed_0x6c: u32,
    pub unnamed_0x70: u32,
    pub unnamed_0x74: u32,
    pub unnamed_0x78: u32,
    pub unnamed_0x7c: u16,
    pub unnamed_0x7e: u16,
}

impl ModelHeader {
    /// Reads the header starting at file offset `0x40` (skipping the
    /// 0x40-byte ASCII build-path comment at the start of the file).
    pub fn read_from<R: Read + Seek>(reader: &mut R) -> Result<Self, ModelError> {
        let mut buf = [0u8; 0x40];
        reader.seek(SeekFrom::Start(0x40))?;
        reader.read_exact(&mut buf)?;

        let version = le_u32(&buf[0x00..0x04]);
        if !KNOWN_VERSIONS.contains(&version) {
            return Err(ModelError::UnknownVersion(version));
        }

        Ok(Self {
            version,
            num_objects: le_u32(&buf[0x04..0x08]),
            num_b: le_u32(&buf[0x08..0x0C]),
            num_objects_dup: le_u32(&buf[0x0C..0x10]),
            num_d: le_u32(&buf[0x10..0x14]),
            objects_offset: le_u32(&buf[0x14..0x18]),
            objects_b_offset: le_u32(&buf[0x18..0x1C]),
            bounds_offset: le_u32(&buf[0x1C..0x20]),
            d_offset: le_u32(&buf[0x20..0x24]),
            unnamed_0x64: le_u32(&buf[0x24..0x28]),
            unnamed_0x68: le_u32(&buf[0x28..0x2C]),
            unnamed_0x6c: le_u32(&buf[0x2C..0x30]),
            unnamed_0x70: le_u32(&buf[0x30..0x34]),
            unnamed_0x74: le_u32(&buf[0x34..0x38]),
            unnamed_0x78: le_u32(&buf[0x38..0x3C]),
            unnamed_0x7c: le_u16(&buf[0x3C..0x3E]),
            unnamed_0x7e: le_u16(&buf[0x3E..0x40]),
        })
    }
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}

fn le_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes.try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::BufReader;
    use std::path::Path;

    #[test]
    fn parses_known_header_from_hexdump() {
        // Bytes for levelE2/objects.ngc, offset 0x40..0x80, captured via
        // `xxd -s 0x40 -l 0x40`.
        let hex = "0d000bf0 ce000000 85000000 ce000000 \
                    0f000000 a0000000 20340000 b0090300 \
                    001d0300 60550000 e85b0000 201f0300 \
                    00000000 00000000 901e0300 83000200";
        let bytes: Vec<u8> = hex
            .split_whitespace()
            .flat_map(|word| (0..4).map(move |i| u8::from_str_radix(&word[i * 2..i * 2 + 2], 16).unwrap()))
            .collect();
        let mut file_bytes = vec![0u8; 0x40];
        file_bytes.extend_from_slice(&bytes);
        let mut cursor = std::io::Cursor::new(file_bytes);

        let header = ModelHeader::read_from(&mut cursor).unwrap();
        assert_eq!(header.version, 0xF00B000D);
        assert_eq!(header.num_objects, 0xCE);
        assert_eq!(header.num_b, 0x85);
        assert_eq!(header.num_objects_dup, 0xCE);
        assert_eq!(header.num_d, 0x0F);
    }

    #[test]
    fn rejects_garbage_version() {
        let mut cursor = std::io::Cursor::new(vec![0u8; 0x80]);
        assert!(matches!(
            ModelHeader::read_from(&mut cursor),
            Err(ModelError::UnknownVersion(0))
        ));
    }

    /// Walks every `objects.ngc` under the user's own extracted disc tree
    /// (never committed to this repo) and confirms every single one parses
    /// with a known version. Skips cleanly if that tree isn't present, so
    /// `cargo test` still passes on a machine without the game data.
    #[test]
    fn every_real_objects_ngc_has_a_known_version() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".to_string());
        let levels_dir = Path::new(&root).join("LEVELS");
        if !levels_dir.is_dir() {
            eprintln!("skipping: {levels_dir:?} not present on this machine");
            return;
        }

        let mut checked = 0;
        for entry in std::fs::read_dir(&levels_dir).unwrap() {
            let level_dir = entry.unwrap().path();
            let model_path = level_dir.join("objects.ngc");
            if !model_path.is_file() {
                continue;
            }
            let file = File::open(&model_path).unwrap();
            let mut reader = BufReader::new(file);
            let header = ModelHeader::read_from(&mut reader)
                .unwrap_or_else(|e| panic!("{model_path:?} failed to parse: {e}"));
            assert_eq!(
                header.num_objects, header.num_objects_dup,
                "{model_path:?}: expected num_objects == num_objects_dup"
            );
            checked += 1;
        }
        assert!(checked > 0, "found no objects.ngc files under {levels_dir:?}");
        eprintln!("validated {checked} real objects.ngc files");
    }
}
