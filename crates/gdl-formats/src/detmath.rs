//! Sines, cosines, arctangents and powers from `libm`, the same to the last
//! bit on every system. Online games run in lockstep, every machine
//! simulating the same ticks, so a Mac and a Windows PC must compute alike;
//! the systems' own maths libraries can round differently
//! (`docs/online.md`). Bevy's `libm` feature does the same for its own
//! maths (glam's rotations).

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
        libm::atan2f(self, x)
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
        libm::atan2(self, x)
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
}
