//! `AUDIO/AUDATPS2.ROM` — the sound catalog: which banks exist, which
//! groups of banks each audio *mode* loads, and every sound's name.
//!
//! Confirmed against the loader that byte-swaps it after reading, the mode
//! and bank lookups, and the name lookup behind `"UNABLE TO FIND SOUND: %s"`
//! (see `docs/audio-format.md`). Little-endian, unlike the banks.
//!
//! ```text
//! 0x00  u32 modes, u32 banks, u32 sounds
//! 0x0C  u32 offsets (from file start) to the mode, bank and sound tables
//! mode  0x2494 bytes: name[16], u32 group count, 32 × 0x124-byte groups:
//!       name[16], u32, u32, u32 bank count, 64 × u32 bank index, u32, u32
//! bank  0x2C bytes: file[16] (AUDIO/<file>.vbk), name[16], u32,
//!       u16 sound count, u16 first sound, u16, u16 (runtime state)
//! sound 0x1C bytes: name[16], u32 id = bank << 16 | call, f32, u32
//! ```

use super::{AudioError, cstr, le_u16, le_u32, slice};

const MODE_LEN: usize = 0x2494;
const GROUP_LEN: usize = 0x124;
const MAX_GROUPS: usize = 32;
const MAX_GROUP_BANKS: usize = 64;
const BANK_LEN: usize = 0x2C;
const SOUND_LEN: usize = 0x1C;

#[derive(Debug, Clone)]
pub struct AudioGroup {
    pub name: String,
    /// Indices into [`AudioCatalog::banks`].
    pub banks: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct AudioMode {
    pub name: String,
    pub groups: Vec<AudioGroup>,
}

#[derive(Debug, Clone)]
pub struct CatalogBank {
    /// File stem: the bank is `AUDIO/<file>.vbk`.
    pub file: String,
    /// The name levels and groups refer to it by (`CASTLE`, `PYRAMID`).
    pub name: String,
    /// Index into [`AudioCatalog::sounds`] of this bank's first sound.
    pub first_sound: usize,
    pub sound_count: usize,
}

impl CatalogBank {
    pub fn path(&self) -> String {
        format!("AUDIO/{}.VBK", self.file)
    }
}

#[derive(Debug, Clone)]
pub struct CatalogSound {
    pub name: String,
    /// Index into [`AudioCatalog::banks`].
    pub bank: usize,
    /// Index into that bank's [`SoundBank::calls`](super::SoundBank::calls).
    pub call: usize,
    /// The call's length in seconds (all its samples, back to back), or -1
    /// when it loops. Matches every call on the disc. The game holds a
    /// voice-queue line for this long (× 60 fields), and counts a started
    /// sound as playing for it (`docs/frontend.md`, "The voice queues").
    pub length: f32,
}

#[derive(Debug, Clone)]
pub struct AudioCatalog {
    pub modes: Vec<AudioMode>,
    pub banks: Vec<CatalogBank>,
    pub sounds: Vec<CatalogSound>,
}

impl AudioCatalog {
    pub fn parse(file: &[u8]) -> Result<Self, AudioError> {
        let h = slice(file, 0, 0x18)?;
        let (num_modes, num_banks, num_sounds) =
            (le_u32(h, 0) as usize, le_u32(h, 4) as usize, le_u32(h, 8) as usize);
        let (modes_at, banks_at, sounds_at) =
            (le_u32(h, 0x0C) as usize, le_u32(h, 0x10) as usize, le_u32(h, 0x14) as usize);

        let bank_table = slice(file, banks_at, num_banks.checked_mul(BANK_LEN).ok_or(AudioError::Invalid("bank count"))?)?;
        let banks: Vec<CatalogBank> = bank_table
            .as_chunks::<BANK_LEN>().0.iter()
            .map(|b| CatalogBank {
                file: cstr(&b[0..0x10]),
                name: cstr(&b[0x10..0x20]),
                sound_count: le_u16(b, 0x24) as usize,
                first_sound: le_u16(b, 0x26) as usize,
            })
            .collect();

        let sound_table =
            slice(file, sounds_at, num_sounds.checked_mul(SOUND_LEN).ok_or(AudioError::Invalid("sound count"))?)?;
        let sounds: Vec<CatalogSound> = sound_table
            .as_chunks::<SOUND_LEN>().0.iter()
            .map(|s| {
                let id = le_u32(s, 0x10);
                CatalogSound {
                    name: cstr(&s[0..0x10]),
                    bank: (id >> 16) as usize,
                    call: (id & 0xFFF) as usize,
                    length: f32::from_bits(le_u32(s, 0x14)),
                }
            })
            .collect();

        let mut modes = Vec::with_capacity(num_modes);
        for m in 0..num_modes {
            let mode = slice(file, modes_at + m * MODE_LEN, MODE_LEN)?;
            let group_count = le_u32(mode, 0x10) as usize;
            if group_count > MAX_GROUPS {
                return Err(AudioError::Invalid("audio mode has more than 32 groups"));
            }
            let groups = (0..group_count)
                .map(|g| {
                    let group = &mode[0x14 + g * GROUP_LEN..][..GROUP_LEN];
                    let count = le_u32(group, 0x18) as usize;
                    if count > MAX_GROUP_BANKS {
                        return Err(AudioError::Invalid("audio group has more than 64 banks"));
                    }
                    let banks = (0..count).map(|i| le_u32(group, 0x1C + i * 4) as usize).collect();
                    Ok(AudioGroup { name: cstr(&group[0..0x10]), banks })
                })
                .collect::<Result<_, _>>()?;
            modes.push(AudioMode { name: cstr(&mode[0..0x10]), groups });
        }

        let catalog = Self { modes, banks, sounds };
        catalog.validate()?;
        Ok(catalog)
    }

    fn validate(&self) -> Result<(), AudioError> {
        let banks = self.banks.len();
        if self.modes.iter().flat_map(|m| &m.groups).flat_map(|g| &g.banks).any(|&b| b >= banks) {
            return Err(AudioError::Invalid("audio group names a bank that doesn't exist"));
        }
        if self.sounds.iter().any(|s| s.bank >= banks) {
            return Err(AudioError::Invalid("sound names a bank that doesn't exist"));
        }
        if self.banks.iter().any(|b| b.first_sound + b.sound_count > self.sounds.len()) {
            return Err(AudioError::Invalid("bank's sounds run past the sound table"));
        }
        Ok(())
    }

    /// Finds a sound by name, like the game: the first match across banks
    /// in table order, comparing at most 15 characters.
    pub fn find_sound(&self, name: &str) -> Option<&CatalogSound> {
        let prefix = |s: &str| s.as_bytes()[..s.len().min(15)].to_vec();
        let key = prefix(name);
        (0..self.banks.len()).flat_map(|b| self.bank_sounds(b)).find(|s| prefix(&s.name) == key)
    }

    /// A bank by the name levels use (`CASTLE`), or by file stem.
    pub fn find_bank(&self, name: &str) -> Option<usize> {
        self.banks.iter().position(|b| b.name.eq_ignore_ascii_case(name) || b.file.eq_ignore_ascii_case(name))
    }

    /// A bank's sounds, in call order.
    pub fn bank_sounds(&self, bank: usize) -> &[CatalogSound] {
        let b = &self.banks[bank];
        &self.sounds[b.first_sound..b.first_sound + b.sound_count]
    }
}
