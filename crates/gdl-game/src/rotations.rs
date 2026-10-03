//! Rotations every machine computes alike (`docs/online.md`, "The same
//! maths everywhere"). glam works on four numbers at a time with the
//! processor's own instructions, written apart for Intel and ARM; most of
//! it comes to the same bits, but a quaternion's length sums its squares in
//! a different order on each, and the Intel `slerp` takes its sines from
//! its own approximation. What the game's ticks can read goes through
//! these instead, a number at a time.

use bevy::prelude::*;
use gdl_formats::detmath::Det;

/// A quaternion from its four numbers, at length one.
fn unit(x: f32, y: f32, z: f32, w: f32) -> Quat {
    let length = (x * x + y * y + z * z + w * w).sqrt();
    Quat::from_xyzw(x / length, y / length, z / length, w / length)
}

/// The rotation `s` of the way from `a` to `b` round the shorter arc
/// (`Quat::slerp`).
pub fn slerp(a: Quat, b: Quat, s: f32) -> Quat {
    let [ax, ay, az, aw] = a.to_array();
    let [mut bx, mut by, mut bz, mut bw] = b.to_array();
    let mut dot = ax * bx + ay * by + az * bz + aw * bw;
    // `b` and `-b` are the same rotation: the one nearer `a` is the short
    // way round.
    if dot < 0.0 {
        (bx, by, bz, bw) = (-bx, -by, -bz, -bw);
        dot = -dot;
    }
    let (ka, kb) = if dot > 1.0 - f32::EPSILON {
        // All but alike: straight between them (no dividing by the sine
        // of nothing).
        (1.0 - s, s)
    } else {
        let angle = dot.dacos();
        let sine = angle.dsin();
        ((angle * (1.0 - s)).dsin() / sine, (angle * s).dsin() / sine)
    };
    unit(ax * ka + bx * kb, ay * ka + by * kb, az * ka + bz * kb, aw * ka + bw * kb)
}

/// The shortest rotation taking the direction `from` to `to`, both of
/// length one (`Quat::from_rotation_arc`).
pub fn arc(from: Vec3, to: Vec3) -> Quat {
    const NEARLY_ONE: f32 = 1.0 - 2.0 * f32::EPSILON;
    let dot = from.dot(to);
    if dot > NEARLY_ONE {
        Quat::IDENTITY
    } else if dot < -NEARLY_ONE {
        // Straight back: half a turn about any axis across it.
        let axis = from.any_orthonormal_vector();
        Quat::from_xyzw(axis.x, axis.y, axis.z, 0.0)
    } else {
        let c = from.cross(to);
        unit(c.x, c.y, c.z, 1.0 + dot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Quat, b: Quat) -> bool {
        // A rotation and its negative are the same one.
        a.abs_diff_eq(b, 1e-5) || a.abs_diff_eq(-b, 1e-5)
    }

    #[test]
    fn slerp_is_glams_to_rounding() {
        let turns = [
            Quat::IDENTITY,
            Quat::from_rotation_y(1.0),
            Quat::from_rotation_y(-2.5),
            Quat::from_euler(EulerRot::YXZ, 0.3, -1.1, 2.0),
            Quat::from_euler(EulerRot::YXZ, 3.0, 0.4, -0.2),
            -Quat::from_rotation_x(0.7),
        ];
        for a in turns {
            for b in turns {
                for s in [0.0, 0.25, 0.5, 0.9, 1.0] {
                    assert!(near(slerp(a, b, s), a.slerp(b, s)), "{a:?} {b:?} {s}");
                }
            }
        }
    }

    #[test]
    fn slerp_ends_on_its_ends() {
        let (a, b) = (Quat::from_rotation_y(0.4), Quat::from_rotation_x(-1.3));
        assert!(near(slerp(a, b, 0.0), a));
        assert!(near(slerp(a, b, 1.0), b));
        assert!(slerp(a, a, 0.5).is_normalized());
    }

    #[test]
    fn arc_is_glams_to_rounding() {
        let directions = [Vec3::X, Vec3::Y, Vec3::NEG_Z, Vec3::new(0.6, 0.0, 0.8), Vec3::new(-0.48, 0.6, 0.64)];
        for from in directions {
            for to in directions {
                assert!(near(arc(from, to), Quat::from_rotation_arc(from, to)), "{from:?} {to:?}");
                assert!((arc(from, to) * from).abs_diff_eq(to, 1e-5));
            }
            assert!((arc(from, -from) * from).abs_diff_eq(-from, 1e-5));
        }
    }
}
