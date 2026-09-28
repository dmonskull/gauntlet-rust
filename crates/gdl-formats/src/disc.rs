//! Nintendo GameCube disc image (.iso/.gcm) header and DOL executable layout.
//!
//! Layout reverse engineered from the public GC-Forever / WiiBrew "Disc" and
//! "Apploader" documentation and confirmed against Gauntlet: Dark Legacy's
//! own disc image (game ID `GUNE5D`): boot.bin's `dol_offset` at 0x420 points
//! at a valid DOL whose entry point Ghidra resolves cleanly at 0x800051fc.

use std::io::{self, Read, Seek, SeekFrom};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DiscError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("disc image too short to contain a boot header")]
    Truncated,
}

#[derive(Debug, Clone)]
pub struct DiscHeader {
    pub game_id: String,
    pub maker_code: String,
    pub disc_number: u8,
    pub disc_version: u8,
    pub title: String,
    pub dol_offset: u32,
    pub fst_offset: u32,
    pub fst_size: u32,
    pub fst_max_size: u32,
}

impl DiscHeader {
    /// Reads boot.bin (the first 0x440 bytes of the disc image).
    pub fn read_from<R: Read + Seek>(reader: &mut R) -> Result<Self, DiscError> {
        let mut header = [0u8; 0x440];
        reader.seek(SeekFrom::Start(0))?;
        reader.read_exact(&mut header).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                DiscError::Truncated
            } else {
                DiscError::Io(e)
            }
        })?;

        let game_id = ascii_str(&header[0..6]);
        let maker_code = ascii_str(&header[6..8]);
        let disc_number = header[0x18];
        let disc_version = header[0x19];
        let title = cstr(&header[0x20..0x60]);
        let dol_offset = be_u32(&header[0x420..0x424]);
        let fst_offset = be_u32(&header[0x424..0x428]);
        let fst_size = be_u32(&header[0x428..0x42C]);
        let fst_max_size = be_u32(&header[0x42C..0x430]);

        Ok(Self {
            game_id,
            maker_code,
            disc_number,
            disc_version,
            title,
            dol_offset,
            fst_offset,
            fst_size,
            fst_max_size,
        })
    }
}

/// A GameCube DOL executable: up to 7 .text and 11 .data sections plus BSS,
/// each independently placed in memory. There is no single "size" field in
/// the header; total size is the max of every section's (offset + size).
#[derive(Debug, Clone)]
pub struct DolHeader {
    pub text_offsets: [u32; 7],
    pub data_offsets: [u32; 11],
    pub text_addresses: [u32; 7],
    pub data_addresses: [u32; 11],
    pub text_sizes: [u32; 7],
    pub data_sizes: [u32; 11],
    pub bss_address: u32,
    pub bss_size: u32,
    pub entry_point: u32,
}

impl DolHeader {
    /// Reads the 0x100-byte DOL header at `dol_offset` within `reader`.
    pub fn read_from<R: Read + Seek>(reader: &mut R, dol_offset: u32) -> Result<Self, DiscError> {
        let mut header = [0u8; 0x100];
        reader.seek(SeekFrom::Start(dol_offset as u64))?;
        reader.read_exact(&mut header)?;

        let text_offsets = be_u32_array::<7>(&header[0x00..0x1C]);
        let data_offsets = be_u32_array::<11>(&header[0x1C..0x48]);
        let text_addresses = be_u32_array::<7>(&header[0x48..0x64]);
        let data_addresses = be_u32_array::<11>(&header[0x64..0x90]);
        let text_sizes = be_u32_array::<7>(&header[0x90..0xAC]);
        let data_sizes = be_u32_array::<11>(&header[0xAC..0xD8]);
        let bss_address = be_u32(&header[0xD8..0xDC]);
        let bss_size = be_u32(&header[0xDC..0xE0]);
        let entry_point = be_u32(&header[0xE0..0xE4]);

        Ok(Self {
            text_offsets,
            data_offsets,
            text_addresses,
            data_addresses,
            text_sizes,
            data_sizes,
            bss_address,
            bss_size,
            entry_point,
        })
    }

    /// Total on-disk size: the furthest (offset + size) across every section.
    pub fn file_size(&self) -> u32 {
        let mut max_end = 0u32;
        for (&offset, &size) in self.text_offsets.iter().zip(self.text_sizes.iter()) {
            if size > 0 {
                max_end = max_end.max(offset + size);
            }
        }
        for (&offset, &size) in self.data_offsets.iter().zip(self.data_sizes.iter()) {
            if size > 0 {
                max_end = max_end.max(offset + size);
            }
        }
        max_end
    }
}

fn ascii_str(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

fn cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().unwrap())
}

fn be_u32_array<const N: usize>(bytes: &[u8]) -> [u32; N] {
    std::array::from_fn(|i| be_u32(&bytes[i * 4..i * 4 + 4]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn make_header(dol_offset: u32) -> Vec<u8> {
        let mut h = vec![0u8; 0x440];
        h[0..6].copy_from_slice(b"GUNE5D");
        h[0x20..0x28].copy_from_slice(b"Gauntlet");
        h[0x420..0x424].copy_from_slice(&dol_offset.to_be_bytes());
        h[0x424..0x428].copy_from_slice(&0x25B200u32.to_be_bytes());
        h[0x428..0x42C].copy_from_slice(&0x11872u32.to_be_bytes());
        h
    }

    #[test]
    fn parses_boot_header_fields() {
        let data = make_header(0x1DA00);
        let mut cursor = Cursor::new(data);
        let header = DiscHeader::read_from(&mut cursor).unwrap();
        assert_eq!(header.game_id, "GUNE5D");
        assert_eq!(header.title, "Gauntlet");
        assert_eq!(header.dol_offset, 0x1DA00);
        assert_eq!(header.fst_offset, 0x25B200);
        assert_eq!(header.fst_size, 0x11872);
    }

    #[test]
    fn rejects_truncated_image() {
        let mut cursor = Cursor::new(vec![0u8; 0x10]);
        assert!(matches!(
            DiscHeader::read_from(&mut cursor),
            Err(DiscError::Truncated)
        ));
    }

    #[test]
    fn computes_dol_file_size_from_max_section_end() {
        let mut h = vec![0u8; 0x100];
        // One text section at offset 0x100, size 0x40.
        h[0x00..0x04].copy_from_slice(&0x100u32.to_be_bytes());
        h[0x90..0x94].copy_from_slice(&0x40u32.to_be_bytes());
        // One data section at offset 0x200, size 0x10 (the true max end).
        h[0x1C..0x20].copy_from_slice(&0x200u32.to_be_bytes());
        h[0xAC..0xB0].copy_from_slice(&0x10u32.to_be_bytes());
        h[0xE0..0xE4].copy_from_slice(&0x100u32.to_be_bytes());

        let mut cursor = Cursor::new(h);
        let dol = DolHeader::read_from(&mut cursor, 0).unwrap();
        assert_eq!(dol.file_size(), 0x210);
        assert_eq!(dol.entry_point, 0x100);
    }
}
