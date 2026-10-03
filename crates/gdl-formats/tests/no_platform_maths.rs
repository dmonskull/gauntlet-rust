//! The maths that differs between systems stays out of `crates/`: online
//! every machine must compute the game alike (`docs/online.md`, "The same
//! maths everywhere"). This is a grep over a fixed set of shapes.
//!
//! - The platform's own functions: their last bits differ (a Mac's and a
//!   Windows PC's sines aren't the same number).
//!   `gdl_formats::detmath::Det` has the same ones from `libm`
//!   (`x.dsin()` …).
//! - The few of glam's that its Intel and ARM versions work out in
//!   different ways (the game's `rotations.rs` has them a number at a
//!   time), and Bevy's frustum test, which sums four products in the
//!   processor's own order (`monsters::on_screen` is the game's).

use std::path::Path;

/// The functions whose results the platform decides.
const PLATFORM: [&str; 26] = [
    ".sin()",
    ".cos()",
    ".tan()",
    ".sin_cos()",
    ".asin()",
    ".acos()",
    ".atan()",
    ".atan2(",
    ".sinh()",
    ".cosh()",
    ".tanh()",
    ".asinh()",
    ".acosh()",
    ".atanh()",
    ".powf(",
    ".powi(",
    ".exp()",
    ".exp2()",
    ".exp_m1()",
    ".ln()",
    ".ln_1p()",
    ".log(",
    ".log10()",
    ".log2()",
    ".cbrt()",
    ".hypot(",
];

/// The vector maths whose results the processor decides.
const PROCESSOR: [&str; 4] = [".slerp(", "from_rotation_arc(", "intersects_sphere(", "intersects_obb("];

fn scan(dir: &Path, hits: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            // The dev tools (`examples/`) never run in a game.
            if path.file_name().is_some_and(|n| n != "target" && n != "examples") {
                scan(&path, hits);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            // The wrappers themselves, and this list.
            if path.ends_with("detmath.rs") || path.ends_with("no_platform_maths.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let mut testing = false;
            for (n, line) in text.lines().enumerate() {
                // A file's tests come last, and may compare with the
                // processor's own.
                testing |= line.trim() == "#[cfg(test)]";
                let code = line.split("//").next().unwrap_or("");
                let processor = PROCESSOR.iter().filter(|_| !testing);
                if let Some(shape) = PLATFORM.iter().chain(processor).find(|s| code.contains(**s)) {
                    hits.push(format!("{}:{}: {shape}: {}", path.display(), n + 1, line.trim()));
                }
            }
        }
    }
}

#[test]
fn crates_use_the_same_maths_everywhere() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut hits = Vec::new();
    scan(crates, &mut hits);
    assert!(hits.is_empty(), "maths that differs between systems (use `detmath::Det`, `rotations.rs`):\n{}", hits.join("\n"));
}
