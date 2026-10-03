//! Sines, cosines, arctangents and powers from `libm`, the same to the last
//! bit on every system. Online games run in lockstep, every machine
//! simulating the same ticks, so a Mac and a Windows PC must compute alike;
//! the systems' own maths libraries can round differently
//! (`docs/online.md`). Bevy's `libm` feature does the same for its own
//! maths (glam's rotations).

/// A float's bits for comparing the game's state between machines: the
/// two zeros alike, and every NaN alike. `f32::max` and `min` may return
/// either of two equal inputs — Rust leaves it open, and the processors
/// differ: for `(-0.0).max(0.0)` an Intel one gives the first, an ARM one
/// always `+0.0` — so equal states can hold zeros of either sign.
pub fn sync_bits(x: f32) -> u32 {
    if x == 0.0 {
        0
    } else if x.is_nan() {
        0x7FC0_0000
    } else {
        x.to_bits()
    }
}

/// [`sync_bits`] for an `f64`.
pub fn sync_bits64(x: f64) -> u64 {
    if x == 0.0 {
        0
    } else if x.is_nan() {
        0x7FF8_0000_0000_0000
    } else {
        x.to_bits()
    }
}

pub trait Det: Sized {
    fn dsin(self) -> Self;
    fn dcos(self) -> Self;
    fn dtan(self) -> Self;
    fn dasin(self) -> Self;
    fn dacos(self) -> Self;
    fn datan(self) -> Self;
    fn datan2(self, x: Self) -> Self;
    fn dsin_cos(self) -> (Self, Self);
    fn dpowf(self, n: Self) -> Self;
    fn dhypot(self, other: Self) -> Self;
    fn dlog10(self) -> Self;
}

impl Det for f32 {
    fn dsin(self) -> f32 {
        libm::sinf(self)
    }
    fn dcos(self) -> f32 {
        libm::cosf(self)
    }
    fn dtan(self) -> f32 {
        libm::tanf(self)
    }
    fn dasin(self) -> f32 {
        libm::asinf(self)
    }
    fn dacos(self) -> f32 {
        libm::acosf(self)
    }
    fn datan(self) -> f32 {
        libm::atanf(self)
    }
    fn datan2(self, x: f32) -> f32 {
        // Adding +0.0 turns −0.0 into +0.0 and changes nothing else: the
        // arctangent of (±0, x < 0) is ±π by the zero's sign, which the
        // systems needn't agree on (see `sync_bits`).
        libm::atan2f(self + 0.0, x + 0.0)
    }
    fn dsin_cos(self) -> (f32, f32) {
        (libm::sinf(self), libm::cosf(self))
    }
    fn dpowf(self, n: f32) -> f32 {
        libm::powf(self, n)
    }
    fn dhypot(self, other: f32) -> f32 {
        libm::hypotf(self, other)
    }
    fn dlog10(self) -> f32 {
        libm::log10f(self)
    }
}

impl Det for f64 {
    fn dsin(self) -> f64 {
        libm::sin(self)
    }
    fn dcos(self) -> f64 {
        libm::cos(self)
    }
    fn dtan(self) -> f64 {
        libm::tan(self)
    }
    fn dasin(self) -> f64 {
        libm::asin(self)
    }
    fn dacos(self) -> f64 {
        libm::acos(self)
    }
    fn datan(self) -> f64 {
        libm::atan(self)
    }
    fn datan2(self, x: f64) -> f64 {
        libm::atan2(self + 0.0, x + 0.0)
    }
    fn dsin_cos(self) -> (f64, f64) {
        (libm::sin(self), libm::cos(self))
    }
    fn dpowf(self, n: f64) -> f64 {
        libm::pow(self, n)
    }
    fn dhypot(self, other: f64) -> f64 {
        libm::hypot(self, other)
    }
    fn dlog10(self) -> f64 {
        libm::log10(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn they_agree_with_the_usual_maths() {
        for x in [-3.0f32, -0.5, 0.0, 0.25, 1.0, 2.5, 6.0] {
            assert!((x.dsin() - x.sin()).abs() < 1e-6);
            assert!((x.dcos() - x.cos()).abs() < 1e-6);
            assert!((x.datan2(1.5) - x.atan2(1.5)).abs() < 1e-6);
        }
        assert_eq!(2.0f32.dpowf(3.0), 8.0);
        assert_eq!(3.0f32.dhypot(4.0), 5.0);
    }

    #[test]
    fn a_zeros_sign_changes_nothing() {
        assert_eq!(sync_bits(-0.0), sync_bits(0.0));
        assert_eq!(sync_bits(f32::NAN), sync_bits(-f32::NAN));
        assert_ne!(sync_bits(1.0), sync_bits(-1.0));
        assert_eq!(sync_bits64(-0.0), sync_bits64(0.0));
        // Straight behind: +π whichever zero the side offset is.
        assert_eq!((-0.0f32).datan2(-1.0).to_bits(), 0.0f32.datan2(-1.0).to_bits());
        assert_eq!((-0.0f64).datan2(-1.0).to_bits(), 0.0f64.datan2(-1.0).to_bits());
        assert_eq!(1.0f32.datan2(-0.0).to_bits(), 1.0f32.datan2(0.0).to_bits());
    }
}
