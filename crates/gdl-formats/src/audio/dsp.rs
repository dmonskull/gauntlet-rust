//! Nintendo GameCube DSP-ADPCM: the codec every sound bank sample and music
//! stream on the disc is stored in.
//!
//! The game never decodes ADPCM itself — it hands the coefficients and the
//! raw frames to the audio DSP (voice format 0 = ADPCM, see
//! `docs/audio-format.md`). The frame layout and the prediction formula here
//! are the hardware's: 8-byte frames of one predictor/scale byte and 14
//! signed 4-bit residuals, predicted from the last two output samples with
//! one of 8 coefficient pairs.
//!
//! Each sample or stream channel is preceded by the standard 0x60-byte
//! big-endian header Nintendo's `DSPADPCM` tool writes (the disc even ships
//! that tool's text dump of one, `AUDIO/sound3.txt`).

use super::{AudioError, be_u16, be_u32, slice};

pub const FRAME_BYTES: usize = 8;
pub const SAMPLES_PER_FRAME: usize = 14;
pub const HEADER_LEN: usize = 0x60;

/// The 0x60-byte DSP-ADPCM header preceding each sample or stream channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DspHeader {
    pub num_samples: u32,
    pub num_nibbles: u32,
    pub sample_rate: u32,
    pub loop_flag: u16,
    /// 0 = ADPCM (the only value on the disc).
    pub format: u16,
    pub loop_start: u32,
    pub loop_end: u32,
    pub current_address: u32,
    /// 8 predictor pairs, `[c1, c2]` for predictor `i` at `2i`, `2i+1`.
    pub coefs: [i16; 16],
    pub gain: u16,
    /// Predictor/scale byte of the first frame.
    pub initial_ps: u16,
    pub hist1: i16,
    pub hist2: i16,
    pub loop_ps: u16,
    pub loop_hist1: i16,
    pub loop_hist2: i16,
}

impl DspHeader {
    pub fn parse(b: &[u8]) -> Result<Self, AudioError> {
        let b = slice(b, 0, HEADER_LEN)?;
        Ok(Self {
            num_samples: be_u32(b, 0x00),
            num_nibbles: be_u32(b, 0x04),
            sample_rate: be_u32(b, 0x08),
            loop_flag: be_u16(b, 0x0C),
            format: be_u16(b, 0x0E),
            loop_start: be_u32(b, 0x10),
            loop_end: be_u32(b, 0x14),
            current_address: be_u32(b, 0x18),
            coefs: std::array::from_fn(|i| be_u16(b, 0x1C + i * 2) as i16),
            gain: be_u16(b, 0x3C),
            initial_ps: be_u16(b, 0x3E),
            hist1: be_u16(b, 0x40) as i16,
            hist2: be_u16(b, 0x42) as i16,
            loop_ps: be_u16(b, 0x44),
            loop_hist1: be_u16(b, 0x46) as i16,
            loop_hist2: be_u16(b, 0x48) as i16,
        })
    }

    /// A decoder primed with this header's coefficients and history.
    pub fn decoder(&self) -> AdpcmDecoder {
        AdpcmDecoder::new(self.coefs, self.hist1, self.hist2)
    }
}

/// Bytes of ADPCM data holding `samples` samples (whole frames).
pub fn bytes_for_samples(samples: usize) -> usize {
    samples.div_ceil(SAMPLES_PER_FRAME) * FRAME_BYTES
}

/// DSP-ADPCM decoder state for one channel.
#[derive(Debug, Clone)]
pub struct AdpcmDecoder {
    coefs: [i16; 16],
    hist1: i32,
    hist2: i32,
}

impl AdpcmDecoder {
    pub fn new(coefs: [i16; 16], hist1: i16, hist2: i16) -> Self {
        Self { coefs, hist1: hist1 as i32, hist2: hist2 as i32 }
    }

    /// Decodes one 8-byte frame into its 14 samples.
    pub fn decode_frame(&mut self, frame: &[u8; FRAME_BYTES]) -> [i16; SAMPLES_PER_FRAME] {
        let ps = frame[0];
        let predictor = ((ps >> 4) & 7) as usize;
        let scale = 1i32 << (ps & 0xF);
        let c1 = self.coefs[predictor * 2] as i32;
        let c2 = self.coefs[predictor * 2 + 1] as i32;
        let mut out = [0i16; SAMPLES_PER_FRAME];
        for (i, o) in out.iter_mut().enumerate() {
            let byte = frame[1 + i / 2];
            let nibble = if i % 2 == 0 { byte >> 4 } else { byte & 0xF };
            let residual = ((nibble as i32) << 28) >> 28; // sign-extend 4 bits
            let predicted = c1 * self.hist1 + c2 * self.hist2;
            let s = (((residual * scale) << 11) + 1024 + predicted) >> 11;
            let s = s.clamp(i16::MIN as i32, i16::MAX as i32);
            self.hist2 = self.hist1;
            self.hist1 = s;
            *o = s as i16;
        }
        out
    }
}

/// Decodes `num_samples` samples of contiguous (single-channel) ADPCM data.
/// Stops early, without error, if `data` runs out.
pub fn decode(data: &[u8], header: &DspHeader, num_samples: usize) -> Vec<i16> {
    let mut decoder = header.decoder();
    let mut out = Vec::with_capacity(num_samples);
    for frame in data.as_chunks::<FRAME_BYTES>().0 {
        if out.len() >= num_samples {
            break;
        }
        let samples = decoder.decode_frame(frame);
        let take = (num_samples - out.len()).min(SAMPLES_PER_FRAME);
        out.extend_from_slice(&samples[..take]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_frame_with_zero_history_is_silent() {
        let mut d = AdpcmDecoder::new([0; 16], 0, 0);
        assert_eq!(d.decode_frame(&[0; 8]), [0; 14]);
    }

    #[test]
    fn residuals_scale_and_sign_extend() {
        // Predictor 0 with zero coefficients: output = residual * scale.
        // Scale exponent 3 → ×8; nibbles 7, -8, 1, -1.
        let mut d = AdpcmDecoder::new([0; 16], 0, 0);
        let s = d.decode_frame(&[0x03, 0x78, 0x1F, 0, 0, 0, 0, 0]);
        assert_eq!(&s[..4], &[56, -64, 8, -8]);
    }

    #[test]
    fn prediction_uses_the_selected_coefficient_pair() {
        // Predictor 1 = (c1 2048, c2 0) = "repeat the last sample" in 11-bit
        // fixed point. With history 100 and zero residuals, it holds at 100.
        let mut coefs = [0i16; 16];
        coefs[2] = 2048;
        let mut d = AdpcmDecoder::new(coefs, 100, 0);
        assert_eq!(d.decode_frame(&[0x10, 0, 0, 0, 0, 0, 0, 0]), [100; 14]);
        // c2 = -2048 alone: s[n] = -s[n-2] — alternates with period 4.
        let mut coefs = [0i16; 16];
        coefs[5] = -2048;
        let mut d = AdpcmDecoder::new(coefs, 0, 50);
        let s = d.decode_frame(&[0x20, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&s[..4], &[-50, 0, 50, 0]);
    }

    #[test]
    fn output_clamps_to_i16() {
        let mut coefs = [0i16; 16];
        coefs[0] = 4096; // 2 × last sample
        let mut d = AdpcmDecoder::new(coefs, 30000, 0);
        assert_eq!(d.decode_frame(&[0; 8])[0], i16::MAX);
    }

    #[test]
    fn decode_stops_at_the_sample_count() {
        let h = DspHeader::parse(&[0u8; HEADER_LEN]).unwrap();
        assert_eq!(decode(&[0u8; 24], &h, 20).len(), 20);
        assert_eq!(decode(&[0u8; 8], &h, 20).len(), 14); // data ran out
        assert_eq!(bytes_for_samples(15), 16);
    }

    #[test]
    fn header_fields_are_big_endian() {
        let mut b = [0u8; HEADER_LEN];
        b[0..4].copy_from_slice(&33572u32.to_be_bytes());
        b[8..12].copy_from_slice(&18000u32.to_be_bytes());
        b[0x1C..0x1E].copy_from_slice(&(-305i16).to_be_bytes());
        b[0x3E..0x40].copy_from_slice(&0x46u16.to_be_bytes());
        let h = DspHeader::parse(&b).unwrap();
        assert_eq!((h.num_samples, h.sample_rate, h.coefs[0], h.initial_ps), (33572, 18000, -305, 0x46));
    }
}
