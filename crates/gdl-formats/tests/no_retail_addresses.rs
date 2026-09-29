//! Retail addresses and decompiler placeholder names stay out of `crates/`
//! (they belong in `docs/`): a standalone runtime resolves nothing against
//! the retail binary, so they'd only be dead weight in shipped code. This
//! is a grep over a fixed set of shapes, nothing more.

use std::path::Path;

fn is_hex(s: &str, len: usize) -> bool {
    s.len() >= len && s.as_bytes()[..len].iter().all(u8::is_ascii_hexdigit)
}

/// `FUN_xxxxxxxx`, `DAT_xxxxxxxx`, `LAB_xxxxxxxx`, or a `0x80xxxxxx` address.
fn offending(line: &str) -> Option<&str> {
    for prefix in ["FUN_", "DAT_", "LAB_"] {
        if let Some(i) = line.find(prefix)
            && is_hex(&line[i + 4..], 8)
        {
            return Some(prefix);
        }
    }
    let lower = line.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("0x80") {
        let rest = &lower[from + i + 4..];
        let digits = rest.bytes().take_while(u8::is_ascii_hexdigit).count();
        if digits == 6 {
            return Some("0x80xxxxxx address");
        }
        from += i + 4;
    }
    None
}

fn scan(dir: &Path, hits: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "target") {
                scan(&path, hits);
            }
        } else if path.extension().is_some_and(|e| e == "rs" || e == "wgsl") {
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                if let Some(what) = offending(line) {
                    hits.push(format!("{}:{}: {what}: {}", path.display(), n + 1, line.trim()));
                }
            }
        }
    }
}

#[test]
fn crates_contain_no_retail_addresses() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut hits = Vec::new();
    scan(crates, &mut hits);
    // This file names the shapes it looks for; exclude its own patterns.
    hits.retain(|h| !h.contains("no_retail_addresses.rs"));
    assert!(hits.is_empty(), "retail addresses belong in docs/, not crates/:\n{}", hits.join("\n"));
}

#[test]
fn detector_catches_each_shape() {
    assert!(offending("see FUN_800c48c0 for").is_some());
    assert!(offending("DAT_802ca298 = 0").is_some());
    assert!(offending("table at 0x80127bb0").is_some());
    assert!(offending("magic 0xF00B000D and 0x8000 flag").is_none());
    assert!(offending("0x80000000 base").is_some());
}
