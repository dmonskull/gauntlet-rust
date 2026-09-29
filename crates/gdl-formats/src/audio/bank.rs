//! `AUDIO/*.VBK` — a sound-effect bank: a list of *calls* (what the game
//! plays by number) followed by the samples the calls sequence.
//!
//! Confirmed against the bank loader in `main.dol` (header check, call-list
//! reader, per-sample reader and the voice code that walks a call; see
//! `docs/audio-format.md`). Everything is big-endian.
//!
//! ```text
//! 0x00  "KNBV"
//! 0x04  u32 call-list size in bytes
//! 0x08  u32 version (low 16 bits 0x106 on every retail bank)
//! 0x0C  u32 number of calls
//! 0x10  u32 number of samples (version 0x106)
//! 0x14  call list
//!       samples, back to back: 0x30-byte "VAGp" header, 0x60-byte DSP
//!       header (when the VAG header's version field is 0x28), ADPCM data
//! ```
//!
//! A call is a run of u16 *steps* — sample index in the low 12 bits, flags
//! in the top 3 — ending at the step with bit 15 set, followed by three u16
//! parameters (volume, duck, priority).

use super::dsp::{self, DspHeader};
use super::{AudioError, be_u16, be_u32, cstr, slice};

pub const MAGIC: &[u8; 4] = b"KNBV";
pub const VERSION: u16 = 0x106;
const VAG_MAGIC: &[u8; 4] = b"VAGp";
const VAG_HEADER_LEN: usize = 0x30;
/// VAG header version field value meaning "a DSP-ADPCM header follows".
const VAG_HAS_DSP_HEADER: u32 = 0x28;

/// Step flag: the last step of the call; three parameter words follow.
pub const STEP_END: u16 = 0x8000;
/// Step flag: where a loop-back jumps to.
pub const STEP_LOOP_START: u16 = 0x4000;
/// Step flag: after this step, jump back to the nearest loop start at or
/// before it (checked before `STEP_END`, so an end step can loop).
pub const STEP_LOOP_BACK: u16 = 0x2000;
const STEP_INDEX: u16 = 0x0FFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallStep {
    /// Index into the bank's samples.
    pub sample: usize,
    pub loop_start: bool,
    pub loop_back: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundCall {
    pub steps: Vec<CallStep>,
    /// 0..=127, scales the requested volume (`volume * requested / 127`).
    pub volume: u16,
    /// How much this call ducks every other playing voice while it plays.
    pub duck: u16,
    /// Voice-stealing priority.
    pub priority: u16,
}

/// A call's playback order: `intro` once, then `looped` forever (empty if
/// the call doesn't loop). Both are sample indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSequence {
    pub intro: Vec<usize>,
    pub looped: Vec<usize>,
}

impl SoundCall {
    /// Walks the steps like the voice code does: next step, or back to the
    /// loop start after a loop-back step, or stop after the end step.
    pub fn sequence(&self) -> Result<CallSequence, AudioError> {
        let mut intro = Vec::new();
        for (i, step) in self.steps.iter().enumerate() {
            if step.loop_back {
                let start = self.steps[..=i]
                    .iter()
                    .rposition(|s| s.loop_start)
                    .ok_or(AudioError::Invalid("loop-back step with no loop start in its call"))?;
                intro.truncate(start);
                let looped = self.steps[start..=i].iter().map(|s| s.sample).collect();
                return Ok(CallSequence { intro, looped });
            }
            intro.push(step.sample);
        }
        Ok(CallSequence { intro, looped: Vec::new() })
    }
}

#[derive(Debug, Clone)]
pub struct BankSample {
    /// Name from the VAG header (the source file's, e.g. `barelgs`).
    pub name: String,
    /// Playback rate from the VAG header.
    pub sample_rate: u32,
    pub dsp: DspHeader,
    /// Raw ADPCM frames.
    pub data: Vec<u8>,
}

impl BankSample {
    /// Samples actually present: the header's count, capped by the data.
    pub fn num_samples(&self) -> usize {
        let whole = self.data.len() / dsp::FRAME_BYTES * dsp::SAMPLES_PER_FRAME;
        (self.dsp.num_samples as usize).min(whole)
    }

    pub fn duration_secs(&self) -> f64 {
        self.num_samples() as f64 / self.sample_rate.max(1) as f64
    }

    /// Mono PCM.
    pub fn decode(&self) -> Vec<i16> {
        dsp::decode(&self.data, &self.dsp, self.num_samples())
    }
}

#[derive(Debug, Clone)]
pub struct SoundBank {
    pub version: u32,
    pub calls: Vec<SoundCall>,
    pub samples: Vec<BankSample>,
}

impl SoundBank {
    pub fn parse(file: &[u8]) -> Result<Self, AudioError> {
        let h = slice(file, 0, 0x14)?;
        if &h[0..4] != MAGIC {
            return Err(AudioError::BadMagic("KNBV"));
        }
        let call_bytes = be_u32(h, 0x04) as usize;
        let version = be_u32(h, 0x08);
        if version as u16 != VERSION {
            return Err(AudioError::UnsupportedVersion(version));
        }
        let num_calls = be_u32(h, 0x0C) as usize;
        let num_samples = be_u32(h, 0x10) as usize;

        let list = slice(file, 0x14, call_bytes)?;
        let words: Vec<u16> = (0..call_bytes / 2).map(|i| be_u16(list, i * 2)).collect();
        let calls = parse_calls(&words, num_calls)?;

        let mut samples = Vec::with_capacity(num_samples);
        let mut at = 0x14 + call_bytes;
        for _ in 0..num_samples {
            let vag = slice(file, at, VAG_HEADER_LEN)?;
            if &vag[0..4] != VAG_MAGIC {
                return Err(AudioError::BadMagic("VAGp"));
            }
            if be_u32(vag, 0x04) != VAG_HAS_DSP_HEADER {
                return Err(AudioError::Invalid("VAG sample without a DSP-ADPCM header"));
            }
            let size = be_u32(vag, 0x0C) as usize;
            let sample_rate = be_u32(vag, 0x10);
            let name = cstr(&vag[0x20..0x30]);
            let dsp = DspHeader::parse(slice(file, at + VAG_HEADER_LEN, dsp::HEADER_LEN)?)?;
            at += VAG_HEADER_LEN + dsp::HEADER_LEN;
            let data = slice(file, at, size)?.to_vec();
            at += size;
            samples.push(BankSample { name, sample_rate, dsp, data });
        }

        let bank = Self { version, calls, samples };
        for call in &bank.calls {
            if let Some(s) = call.steps.iter().find(|s| s.sample >= bank.samples.len()) {
                return Err(AudioError::BadSampleIndex(s.sample, bank.samples.len()));
            }
        }
        Ok(bank)
    }
}

fn parse_calls(words: &[u16], count: usize) -> Result<Vec<SoundCall>, AudioError> {
    let mut calls = Vec::with_capacity(count);
    let mut i = 0;
    let word = |i: usize| words.get(i).copied().ok_or(AudioError::Invalid("call list ends mid-call"));
    for _ in 0..count {
        let mut steps = Vec::new();
        loop {
            let w = word(i)?;
            i += 1;
            steps.push(CallStep {
                sample: (w & STEP_INDEX) as usize,
                loop_start: w & STEP_LOOP_START != 0,
                loop_back: w & STEP_LOOP_BACK != 0,
            });
            if w & STEP_END != 0 {
                break;
            }
        }
        calls.push(SoundCall { steps, volume: word(i)?, duck: word(i + 1)?, priority: word(i + 2)? });
        i += 3;
    }
    if i != words.len() {
        return Err(AudioError::Invalid("call list size doesn't match its calls"));
    }
    Ok(calls)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(words: &[u16]) -> SoundCall {
        let mut all = words.to_vec();
        all.extend([0x78, 0, 0x8032]);
        parse_calls(&all, 1).unwrap().remove(0)
    }

    #[test]
    fn single_sample_call() {
        let c = call(&[0x8005]);
        assert_eq!((c.volume, c.duck, c.priority), (0x78, 0, 0x8032));
        let seq = c.sequence().unwrap();
        assert_eq!((seq.intro, seq.looped), (vec![5], vec![]));
    }

    #[test]
    fn a_self_looping_sample_loops_forever() {
        let seq = call(&[0xE006]).sequence().unwrap();
        assert_eq!((seq.intro, seq.looped), (vec![], vec![6]));
    }

    #[test]
    fn intro_then_loop_section() {
        // 1, then [2, 3] repeating; 4 is never reached.
        let seq = call(&[0x0001, 0x4002, 0x2003, 0x8004]).sequence().unwrap();
        assert_eq!((seq.intro, seq.looped), (vec![1], vec![2, 3]));
    }

    #[test]
    fn loop_back_without_start_is_an_error() {
        assert!(call(&[0xA001]).sequence().is_err());
    }

    #[test]
    fn call_list_must_be_consumed_exactly() {
        assert!(parse_calls(&[0x8001, 1, 2, 3, 0], 1).is_err());
        assert!(parse_calls(&[0x8001, 1], 1).is_err());
    }

    #[test]
    fn rejects_other_files() {
        assert!(matches!(SoundBank::parse(&[0u8; 0x20]), Err(AudioError::BadMagic(_))));
    }
}
