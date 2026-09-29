//! Game audio: sound-effect banks (`AUDIO/*.VBK`), the sound catalog
//! (`AUDIO/AUDATPS2.ROM`) and music streams (`STREAMS/*.ads`), all
//! DSP-ADPCM. See `docs/audio-format.md` for the evidence.

pub mod bank;
pub mod catalog;
pub mod dsp;
pub mod stream;

pub use bank::{BankSample, CallSequence, CallStep, SoundBank, SoundCall};
pub use catalog::{AudioCatalog, AudioGroup, AudioMode, CatalogBank, CatalogSound};
pub use dsp::{AdpcmDecoder, DspHeader};
pub use stream::{AdsSamples, AdsStream};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("file is truncated: needed bytes 0x{0:X}..0x{1:X}")]
    Truncated(usize, usize),
    #[error("not a {0} file")]
    BadMagic(&'static str),
    #[error("unsupported bank version 0x{0:08X}")]
    UnsupportedVersion(u32),
    #[error("unsupported stream codec 0x{0:X}")]
    UnsupportedCodec(u32),
    #[error("call plays sample {0}, but the bank has {1}")]
    BadSampleIndex(usize, usize),
    #[error("{0}")]
    Invalid(&'static str),
}

fn slice(b: &[u8], at: usize, len: usize) -> Result<&[u8], AudioError> {
    let end = at.checked_add(len).ok_or(AudioError::Truncated(at, usize::MAX))?;
    b.get(at..end).ok_or(AudioError::Truncated(at, end))
}

fn be_u16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be_u32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

fn le_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// A NUL-terminated name in a fixed field (the tools left junk after the NUL).
fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}
