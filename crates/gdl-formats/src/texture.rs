//! `textures.ngc` — texture pixel data, addressed by [`MaterialBinding`]s.
//!
//! There is no file header: each binding carries an offset, size and a
//! format selector byte. The selector is decoded exactly as the game does
//! when it builds GX texture objects at load time (`docs/textures-ngc-format.md`).
//! It names **PS2 pixel-storage modes** (`PSMT4 = 0x14`, `PSMT8 = 0x13`,
//! `PSMCT16 = 0x02`) through three small lookup tables, which the game maps
//! to GameCube formats: 4-bit → GX `CI4`, 8-bit → GX `CI8`,
//! 16-bit → GX `RGB5A3`. Palettes are big-endian RGB5A3 and sit in front of
//! the pixels; pixel data is in GX's tiled layout.

use thiserror::Error;

use crate::model::MaterialBinding;

#[derive(Debug, Error)]
pub enum TextureError {
    #[error("binding has no texture (zero width or height)")]
    Untextured,
    #[error("format selector 0x{0:02X} is outside the game's own tables")]
    UnsupportedFormat(u8),
    #[error("texture data runs past the end of textures.ngc")]
    Truncated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormat {
    /// GX CI4 with a 16-entry palette; pixels start `palette_skip` bytes in.
    Ci4 { palette_skip: usize },
    /// GX CI8 with a 256-entry palette; pixels start `palette_skip` bytes in.
    Ci8 { palette_skip: usize },
    /// GX CI4/CI8 using the game's global palettes instead of their own.
    /// The game generates those at runtime as RGB5A3 `(i × 0x800) |
    /// 0xFFF` (16 entries) and `(i × 0x80) | 0xFFF` (256): white with a
    /// 3-bit alpha ramp. Lightmaps use this, carrying intensity in alpha.
    SharedPaletteCi4,
    SharedPaletteCi8,
    /// GX RGB5A3, 16 bits per texel.
    Rgb5a3,
}

impl TextureFormat {
    /// Decodes a binding's format selector byte the way the game does.
    pub fn from_selector(selector: u8) -> Result<Self, TextureError> {
        let hi = selector >> 4;
        if hi & 8 != 0 {
            return Ok(if hi == 8 { Self::SharedPaletteCi8 } else { Self::SharedPaletteCi4 });
        }
        match hi {
            // Direct formats via the game's third table; only its PSMCT16
            // entries (selectors 0 and 1) produce a defined texel size.
            0 if selector & 7 <= 1 => Ok(Self::Rgb5a3),
            1 => Ok(Self::Ci4 { palette_skip: 32 }),
            2 => Ok(Self::Ci4 { palette_skip: 64 }),
            3 => Ok(Self::Ci8 { palette_skip: 512 }),
            4 => Ok(Self::Ci8 { palette_skip: 1024 }),
            // 5..7 index past the end of the game's 5-entry tables, and the
            // other direct formats leave the texel size uninitialised there.
            _ => Err(TextureError::UnsupportedFormat(selector)),
        }
    }
}

/// Decoded texture: tightly packed RGBA8, row-major, top row first.
#[derive(Debug, Clone)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    pub fn has_transparency(&self) -> bool {
        self.pixels.as_chunks::<4>().0.iter().any(|p| p[3] < 255)
    }
}

/// Decodes the texture `binding` points at inside `textures` (the whole
/// sibling `textures.ngc`).
pub fn decode(textures: &[u8], binding: &MaterialBinding) -> Result<RgbaImage, TextureError> {
    if !binding.is_textured() {
        return Err(TextureError::Untextured);
    }
    let format = TextureFormat::from_selector(binding.format)?;
    let (w, h) = (binding.width as usize, binding.height as usize);
    let data = textures.get(binding.texture_offset as usize..).ok_or(TextureError::Truncated)?;

    let pixels = match format {
        TextureFormat::Ci4 { palette_skip } => {
            let palette = palette(data, 16)?;
            let texels = data.get(palette_skip..).ok_or(TextureError::Truncated)?;
            untile(w, h, 8, 8, texels, 4, |i| palette[nibble(texels, i) as usize])?
        }
        TextureFormat::Ci8 { palette_skip } => {
            let palette = palette(data, 256)?;
            let texels = data.get(palette_skip..).ok_or(TextureError::Truncated)?;
            untile(w, h, 8, 4, texels, 8, |i| palette[texels[i] as usize])?
        }
        TextureFormat::SharedPaletteCi4 => {
            let palette = shared_palette(16, 0x800);
            untile(w, h, 8, 8, data, 4, |i| palette[nibble(data, i) as usize])?
        }
        TextureFormat::SharedPaletteCi8 => {
            let palette = shared_palette(256, 0x80);
            untile(w, h, 8, 4, data, 8, |i| palette[data[i] as usize])?
        }
        TextureFormat::Rgb5a3 => {
            untile(w, h, 4, 4, data, 16, |i| rgb5a3(u16::from_be_bytes([data[i * 2], data[i * 2 + 1]])))?
        }
    };
    Ok(RgbaImage { width: w as u32, height: h as u32, pixels })
}

fn palette(data: &[u8], entries: usize) -> Result<Vec<[u8; 4]>, TextureError> {
    let raw = data.get(..entries * 2).ok_or(TextureError::Truncated)?;
    Ok(raw.as_chunks::<2>().0.iter().map(|&c| rgb5a3(u16::from_be_bytes(c))).collect())
}

/// The global palettes the game builds at runtime.
fn shared_palette(entries: u16, step: u16) -> Vec<[u8; 4]> {
    (0..entries).map(|i| rgb5a3((i * step) | 0xFFF)).collect()
}

/// 4-bit texel `i`, high nibble first (GX order).
fn nibble(data: &[u8], i: usize) -> u8 {
    let byte = data[i / 2];
    if i.is_multiple_of(2) { byte >> 4 } else { byte & 0xF }
}

/// GX RGB5A3: top bit set → opaque RGB555, clear → ARGB3444.
pub fn rgb5a3(v: u16) -> [u8; 4] {
    if v & 0x8000 != 0 {
        let c = |s: u16| {
            let x = ((v >> s) & 0x1F) as u8;
            x << 3 | x >> 2
        };
        [c(10), c(5), c(0), 255]
    } else {
        let a = ((v >> 12) & 7) as u8;
        let c = |s: u16| ((v >> s) & 0xF) as u8 * 17;
        [c(8), c(4), c(0), a << 5 | a << 2 | a >> 1]
    }
}

/// Walks GX's tiled layout (tiles row-major, texels row-major within a
/// tile; dimensions padded up to whole tiles) calling `texel(i)` with the
/// i-th stored texel index.
fn untile(
    w: usize,
    h: usize,
    tile_w: usize,
    tile_h: usize,
    data: &[u8],
    bits_per_texel: usize,
    texel: impl Fn(usize) -> [u8; 4],
) -> Result<Vec<u8>, TextureError> {
    let padded = w.div_ceil(tile_w) * tile_w * h.div_ceil(tile_h) * tile_h;
    if data.len() * 8 < padded * bits_per_texel {
        return Err(TextureError::Truncated);
    }
    let mut out = vec![0u8; w * h * 4];
    let mut i = 0;
    for ty in (0..h).step_by(tile_h) {
        for tx in (0..w).step_by(tile_w) {
            for y in ty..ty + tile_h {
                for x in tx..tx + tile_w {
                    if x < w && y < h {
                        out[(y * w + x) * 4..][..4].copy_from_slice(&texel(i));
                    }
                    i += 1;
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelFile;

    #[test]
    fn rgb5a3_both_modes() {
        assert_eq!(rgb5a3(0xFFFF), [255, 255, 255, 255]);
        assert_eq!(rgb5a3(0x8000 | 31 << 10), [255, 0, 0, 255]);
        assert_eq!(rgb5a3(0x7F00), [255, 0, 0, 255]); // a=7, r=15
        assert_eq!(rgb5a3(0x0000), [0, 0, 0, 0]);
    }

    #[test]
    fn shared_palettes_are_a_white_alpha_ramp() {
        let p = shared_palette(16, 0x800);
        assert_eq!(p[0], [255, 255, 255, 0]);
        assert_eq!(p[1], [255, 255, 255, 0]); // bit 11 is part of red, already 0xF
        assert_eq!(p[15], [255, 255, 255, 255]);
        assert_eq!(p[7][3], rgb5a3(0x3FFF)[3]); // alpha level 3
        assert_eq!(shared_palette(256, 0x80)[255], [255, 255, 255, 255]);
    }

    #[test]
    fn selectors_match_the_games_tables() {
        use TextureFormat::*;
        assert_eq!(TextureFormat::from_selector(0x32).unwrap(), Ci8 { palette_skip: 512 });
        assert_eq!(TextureFormat::from_selector(0x10).unwrap(), Ci4 { palette_skip: 32 });
        assert_eq!(TextureFormat::from_selector(0x92).unwrap(), SharedPaletteCi4);
        assert_eq!(TextureFormat::from_selector(0x00).unwrap(), Rgb5a3);
        assert!(TextureFormat::from_selector(0x5C).is_err());
    }

    #[test]
    fn ci8_untiles_two_tiles() {
        // 16x4 CI8: two 8x4 tiles; palette index = tile number.
        let mut tex = vec![0u8; 512];
        tex[2..4].copy_from_slice(&0xFC00u16.to_be_bytes()); // index 1 = red
        tex.extend(std::iter::repeat_n(0u8, 32));
        tex.extend(std::iter::repeat_n(1u8, 32));
        let b = MaterialBinding { format: 0x30, flags_raw: 0, texture_offset: 0, width: 16, height: 4 };
        let img = decode(&tex, &b).unwrap();
        assert_eq!(&img.pixels[0..4], &rgb5a3(0)[..]);
        assert_eq!(&img.pixels[8 * 4..8 * 4 + 4], &[255, 0, 0, 255]);
    }

    /// Every textured binding on the disc either decodes, or uses one of the
    /// selectors the game itself can't handle.
    #[test]
    fn every_real_texture_decodes() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let dir = std::path::Path::new(&root).join("LEVELS");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipping: {dir:?} not present");
            return;
        };
        let (mut ok, mut unsupported) = (0, 0);
        for level in entries.flatten().map(|e| e.path()) {
            let (Ok(objects), Ok(textures)) =
                (std::fs::read(level.join("objects.ngc")), std::fs::read(level.join("textures.ngc")))
            else {
                continue;
            };
            let model = ModelFile::parse(&objects).unwrap();
            for b in model.bindings.iter().filter(|b| b.is_textured()) {
                match decode(&textures, b) {
                    Ok(img) => {
                        assert_eq!(img.pixels.len(), (img.width * img.height * 4) as usize);
                        ok += 1;
                    }
                    Err(TextureError::UnsupportedFormat(_)) => unsupported += 1,
                    Err(e) => panic!("{level:?}: {e} ({b:?})"),
                }
            }
        }
        eprintln!("{ok} textures decoded, {unsupported} with out-of-table selectors");
        assert!(ok > 8000 || ok == 0);
    }
}
