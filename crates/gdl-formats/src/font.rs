//! `FONTS/*.FNT` — the game's bitmap fonts (`docs/frontend.md`).
//!
//! A font file is only metrics; the glyph pixels are in a texture with the
//! font's name (`FONT32`, `8HIFONTS`… in `STATIC/objects.ngc`), 2-bit alpha
//! on white, so text takes whatever colour it's drawn with.
//!
//! Layout, little-endian: a 12-byte header `{u32 name (0 on disc, the
//! loader writes the name pointer), u32 flags, u32 glyphs (0 on disc)}`,
//! then 16-byte glyphs `{u32 code, u32 width, u32 x, u32 y}` up to one
//! whose code is 0. The flags' low byte is the height every glyph shares;
//! bit `0x100` marks a digits-only font (`.` and `-` draw as `:` and `;`,
//! other ASCII is skipped). A glyph covers `x..x+width` × `y..y+height`
//! of the texture, and the pen moves on by its width — there is no other
//! spacing.
//!
//! The game keeps 13 fonts in fixed slots ([`FONT_SLOTS`]); the text ROM's
//! `FONT` chunk names the fonts its string groups use, matched against the
//! slot names exactly (case-sensitive; a name that matches nothing falls
//! back to slot 0).

use thiserror::Error;

use crate::chunk::le_u32;

#[derive(Debug, Error)]
pub enum FontError {
    #[error("font file is {0} bytes; the header alone is 12")]
    Truncated(usize),
    #[error("glyph {index} (code {code}) is {width} wide")]
    BadGlyph { index: usize, code: u32, width: u32 },
}

/// One character's rectangle in the font texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    pub code: u32,
    pub width: u32,
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone)]
pub struct FontFile {
    /// Height of every glyph, and the line step.
    pub height: u32,
    /// Flag `0x100`: a digits-only font.
    pub digits_only: bool,
    pub glyphs: Vec<Glyph>,
}

/// Header bytes before the first glyph.
const HEADER: usize = 12;
const GLYPH: usize = 16;
/// A glyph wider than the widest font texture (256) is a corrupt record.
const MAX_WIDTH: u32 = 256;

impl FontFile {
    pub fn parse(data: &[u8]) -> Result<Self, FontError> {
        if data.len() < HEADER {
            return Err(FontError::Truncated(data.len()));
        }
        let flags = le_u32(data, 4);
        let mut glyphs = Vec::new();
        for (index, g) in data[HEADER..].chunks_exact(GLYPH).enumerate() {
            let glyph = Glyph { code: le_u32(g, 0), width: le_u32(g, 4), x: le_u32(g, 8), y: le_u32(g, 12) };
            if glyph.code == 0 {
                break;
            }
            if glyph.width > MAX_WIDTH {
                return Err(FontError::BadGlyph { index, code: glyph.code, width: glyph.width });
            }
            glyphs.push(glyph);
        }
        Ok(Self { height: flags & 0xFF, digits_only: flags & 0x100 != 0, glyphs })
    }

    /// The glyph the game draws for character `code` (the loader files each
    /// glyph under its code; a zero-width entry draws nothing).
    pub fn glyph(&self, code: u32) -> Option<&Glyph> {
        self.glyphs.iter().rev().find(|g| g.code == code && g.width > 0)
    }

    /// Lays out one line (no `\n` handling; see [`split_lines`]) the way
    /// the game's text blitter walks it: glyphs advance by their width, a
    /// space with no glyph advances by the slot's space width, `*` before a
    /// capital letter is an inline button icon one line high, and
    /// characters the font lacks are skipped.
    pub fn layout(&self, text: &[u8], space_width: u32) -> Vec<Placed> {
        let mut out = Vec::new();
        let mut pen = 0u32;
        let mut i = 0;
        while i < text.len() {
            let mut c = text[i] as u32;
            i += 1;
            if c == u32::from(b'*') && text.get(i).is_some_and(u8::is_ascii_uppercase) {
                out.push(Placed::Icon { letter: text[i], x: pen, size: self.height });
                pen += self.height;
                i += 1;
                continue;
            }
            if self.digits_only {
                if c >= 0x80 {
                    // A colour byte, then the character.
                    let Some(&next) = text.get(i) else { break };
                    c = next as u32;
                    i += 1;
                } else if !(b'0'..=b'9').contains(&(c as u8)) {
                    c = match c as u8 {
                        b'.' => u32::from(b':'),
                        b'-' => u32::from(b';'),
                        _ => continue,
                    };
                }
            }
            match self.glyph(c) {
                Some(g) => {
                    out.push(Placed::Glyph { glyph: *g, x: pen });
                    pen += g.width;
                }
                None if c == u32::from(b' ') => pen += space_width,
                None => {}
            }
        }
        out.push(Placed::End { x: pen });
        out
    }

    /// Width of one line in texels at scale 1.
    pub fn width(&self, text: &[u8], space_width: u32) -> u32 {
        match self.layout(text, space_width).last() {
            Some(Placed::End { x }) => *x,
            _ => 0,
        }
    }
}

/// What [`FontFile::layout`] puts where (x in texels from the line start).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placed {
    Glyph { glyph: Glyph, x: u32 },
    /// `*X`: button icon `X`, `size` square.
    Icon { letter: u8, x: u32, size: u32 },
    /// Where the pen ends: the line's width.
    End { x: u32 },
}

/// The game's multi-line text split: at `\n`, at most 16 lines.
pub fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n').take(16)
}

/// One of the game's 13 font slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontSlot {
    /// `FONTS/<name>.fnt`, and the texture's name.
    pub name: &'static str,
    /// How far a space moves the pen when the font has no space glyph.
    pub space_width: u32,
    /// Glyph records the loader reads at most.
    pub max_glyphs: usize,
    /// Glyphs per texture page: glyph `n` is on the texture `n / page`
    /// bindings after the named one (only the Japanese fonts have more
    /// than one page).
    pub page_glyphs: usize,
}

const fn slot(name: &'static str, space_width: u32) -> FontSlot {
    FontSlot { name, space_width, max_glyphs: 0x80, page_glyphs: 0x80 }
}

/// The fonts the game loads at boot, in slot order, with each one's space
/// width (`docs/frontend.md`). Slot 0 is built into the executable — its
/// copy is byte-for-byte `FONTS/FONT8X8.FNT`.
pub const FONT_SLOTS: [FontSlot; 13] = [
    slot("font8x8", 8),
    slot("8Hifonts", 8),
    slot("bars", 4),
    slot("arrows", 8),
    slot("score", 9),
    slot("scoratt", 12),
    slot("font32", 16),
    slot("initials", 12),
    slot("scoratt8", 8),
    slot("namefont", 8),
    FontSlot { name: "kanji10a", space_width: 10, max_glyphs: 0x100, page_glyphs: 0xD2 },
    FontSlot { name: "kanji10b", space_width: 10, max_glyphs: 0x100, page_glyphs: 0xD2 },
    FontSlot { name: "kanji20a", space_width: 20, max_glyphs: 0x100, page_glyphs: 100 },
];

/// Named slots the front end uses.
pub const FONT8X8: usize = 0;
pub const FONT_8HI: usize = 1;
pub const FONT32: usize = 6;
pub const INITIALS: usize = 7;

/// The slot the game gives a font named in a text ROM's `FONT` chunk:
/// exact name match, else slot 0.
pub fn slot_for_rom_font(name: &str) -> usize {
    FONT_SLOTS.iter().position(|s| s.name == name).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(height: u32, glyphs: &[(u8, u32, u32, u32)]) -> FontFile {
        FontFile {
            height,
            digits_only: false,
            glyphs: glyphs.iter().map(|&(c, width, x, y)| Glyph { code: c as u32, width, x, y }).collect(),
        }
    }

    #[test]
    fn layout_follows_the_blitter() {
        let f = font(10, &[(b'A', 11, 0, 0), (b'B', 10, 11, 0)]);
        let placed = f.layout(b"AB A?", 6);
        assert_eq!(placed[0], Placed::Glyph { glyph: f.glyphs[0], x: 0 });
        assert_eq!(placed[1], Placed::Glyph { glyph: f.glyphs[1], x: 11 });
        // Space from the slot, `?` missing: skipped without moving the pen.
        assert_eq!(placed[2], Placed::Glyph { glyph: f.glyphs[0], x: 27 });
        assert_eq!(placed.last(), Some(&Placed::End { x: 38 }));
        assert_eq!(f.width(b"*XA", 6), 21, "an icon is one line high and wide");
    }

    #[test]
    fn digits_only_fonts_map_punctuation() {
        let mut f = font(8, &[(b'1', 8, 0, 0), (b':', 4, 8, 0)]);
        f.digits_only = true;
        assert_eq!(f.width(b"1.x1", 0), 20);
    }

    #[test]
    fn rom_fonts_match_slots_exactly() {
        assert_eq!(slot_for_rom_font("font32"), FONT32);
        assert_eq!(slot_for_rom_font("initials"), INITIALS);
        // ENGLISH.ROM's first font doesn't match `8Hifonts`: the game falls
        // back to slot 0.
        assert_eq!(slot_for_rom_font("8Hi_fonts5"), FONT8X8);
    }

    /// Every font on the disc parses, and every glyph of every font whose
    /// texture is on the disc lies inside that texture.
    #[test]
    fn every_real_font_parses_and_fits_its_texture() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let root = std::path::Path::new(&root);
        let Ok(entries) = std::fs::read_dir(root.join("FONTS")) else {
            eprintln!("skipping: {root:?}/FONTS not present");
            return;
        };
        // Texture sizes by upper-case name, from every model file that has
        // font textures.
        let mut sizes = std::collections::HashMap::new();
        for dir in ["STATIC", "CREDITS", "SELECT"] {
            let Ok(bytes) = std::fs::read(root.join(dir).join("objects.ngc")) else { continue };
            let model = crate::ModelFile::parse(&bytes).unwrap();
            for t in &model.texture_names {
                sizes.entry(t.name.to_ascii_uppercase()).or_insert((t.width as u32, t.height as u32));
            }
        }
        let mut fonts = 0;
        let mut checked = 0;
        for path in entries.flatten().map(|e| e.path()) {
            let data = std::fs::read(&path).unwrap();
            let font = FontFile::parse(&data).unwrap_or_else(|e| panic!("{path:?}: {e}"));
            assert!(font.height > 0 && !font.glyphs.is_empty(), "{path:?}");
            fonts += 1;
            let name = path.file_stem().unwrap().to_string_lossy().to_ascii_uppercase();
            let Some(&(w, h)) = sizes.get(&name) else {
                eprintln!("{name}: no texture on the disc");
                continue;
            };
            if name.starts_with("KANJI") {
                // The US disc carries 32x32 placeholders for the Japanese
                // fonts' pages.
                continue;
            }
            for g in &font.glyphs {
                assert!(g.x + g.width <= w && g.y + font.height <= h, "{name}: glyph {g:?} outside {w}x{h}");
            }
            checked += 1;
        }
        assert!(fonts >= 16, "found {fonts} fonts");
        eprintln!("{fonts} fonts parsed, {checked} checked against their textures");
        if let Some(slot0) = std::fs::read(root.join("FONTS/FONT8X8.FNT")).ok() {
            let f = FontFile::parse(&slot0).unwrap();
            assert_eq!(f.height, 9);
            assert_eq!(f.glyph(u32::from(b'A')).map(|g| g.width), Some(8));
        }
        let text = std::fs::read(root.join("TEXT/ENGLISH.ROM")).unwrap();
        let rom = crate::text::TextRom::parse(&text).unwrap();
        let slots: Vec<usize> = rom.fonts.iter().map(|n| slot_for_rom_font(n)).collect();
        assert_eq!(slots, [FONT8X8, FONT32, INITIALS]);
    }
}
