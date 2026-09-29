//! Locates and reads a user's own copy of Gauntlet: Dark Legacy (GameCube).
//!
//! Point [`GameInstall::locate`] at whatever you have — a `.iso`/`.gcm` disc
//! image, an extracted disc folder in any of the common layouts, or the
//! extracted `main.dol` — and it finds the game data, checks it really is
//! this game, and exposes every file through one read-only API keyed by the
//! game's own paths (`LEVELS/levelA1/objects.ngc`, case-insensitive).
//!
//! Nothing here ever writes to the game's files.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use gdl_formats::{Disc, DiscHeader, FstError};
use thiserror::Error;

/// Game IDs this runtime has been reverse engineered against. Other regional
/// releases are likely compatible but unverified, so they load with a warning
/// rather than being refused.
pub const VERIFIED_GAME_IDS: [&str; 1] = ["GUNE5D"];

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("'{0}' does not exist")]
    Missing(PathBuf),
    #[error(
        "'{0}' is a Dolphin-compressed .{1} image, which isn't supported yet. \
         In Dolphin, right-click the game → Convert File… → Format: ISO, then point at the .iso"
    )]
    CompressedImage(PathBuf, String),
    #[error("'{0}' is not a Gauntlet: Dark Legacy disc image ({1})")]
    NotThisGame(PathBuf, String),
    #[error(
        "couldn't find Gauntlet: Dark Legacy game data in '{0}'. Point at the disc image \
         (.iso/.gcm), the extracted disc folder, or its main.dol"
    )]
    NoGameData(PathBuf),
    #[error("failed to read the disc image: {0}")]
    Disc(#[from] FstError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("'{0}' is not in the game data")]
    NotFound(String),
}

enum Source {
    Disc { disc: Disc, prefix: String },
    Directory { index: HashMap<String, PathBuf> },
}

/// A located, validated copy of the game.
pub struct GameInstall {
    /// Where the user pointed us (what we'd remember for next launch).
    pub origin: PathBuf,
    /// `GUNE5D` etc., when the source carries a boot header.
    pub game_id: Option<String>,
    pub title: Option<String>,
    /// Level folder names under `LEVELS/` that have model data, sorted.
    pub levels: Vec<String>,
    /// Set when the game ID isn't one we've verified against.
    pub warning: Option<String>,
    source: Source,
    /// Every game file path (original casing), relative to the data root.
    files: Vec<String>,
}

impl fmt::Debug for GameInstall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameInstall")
            .field("origin", &self.origin)
            .field("game_id", &self.game_id)
            .field("source", &self.source_kind())
            .field("files", &self.files.len())
            .field("levels", &self.levels.len())
            .finish()
    }
}

impl GameInstall {
    pub fn locate(path: impl AsRef<Path>) -> Result<Self, InstallError> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(InstallError::Missing(path.to_path_buf()));
        }
        if path.is_dir() {
            return Self::from_directory(path, path);
        }

        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "rvz" | "wia" | "gcz" | "ciso" | "wbfs" | "nkit" => {
                Err(InstallError::CompressedImage(path.to_path_buf(), ext))
            }
            "dol" | "bin" => {
                // main.dol / boot.bin inside an extracted tree: `.../sys/main.dol`
                // (Dolphin layout) or `.../main.dol`. Search upward from it.
                let mut dir = path.parent();
                for _ in 0..3 {
                    let Some(d) = dir else { break };
                    if let Ok(install) = Self::from_directory(d, path) {
                        return Ok(install);
                    }
                    dir = d.parent();
                }
                Err(InstallError::NoGameData(path.to_path_buf()))
            }
            _ => Self::from_disc(path),
        }
    }

    fn from_disc(path: &Path) -> Result<Self, InstallError> {
        let disc = Disc::open(path).map_err(|e| match e {
            FstError::Malformed(why) => {
                InstallError::NotThisGame(path.to_path_buf(), format!("no valid GameCube filesystem: {why}"))
            }
            other => other.into(),
        })?;

        let paths = disc.fst.paths().to_vec();
        let prefix = paths
            .iter()
            .find_map(|p| data_root_prefix(p))
            .ok_or_else(|| {
                InstallError::NotThisGame(
                    path.to_path_buf(),
                    format!("disc '{}' ({}) has no LEVELS data", disc.header.title, disc.header.game_id),
                )
            })?;
        let files = paths
            .iter()
            .filter_map(|p| p.strip_prefix(prefix.as_str()).map(str::to_string))
            .collect::<Vec<_>>();
        let (game_id, title, warning) = identify(&disc.header);

        Ok(Self {
            origin: path.to_path_buf(),
            game_id: Some(game_id),
            title: Some(title),
            levels: levels_in(&files),
            warning,
            source: Source::Disc { disc, prefix },
            files,
        })
    }

    fn from_directory(dir: &Path, origin: &Path) -> Result<Self, InstallError> {
        // Layouts seen in the wild, relative to what the user pointed at:
        // the data folder itself, `Gauntlet/` (plain extraction), and
        // Dolphin's "Extract Entire Disc" `files/` + `files/Gauntlet/`.
        let candidates = [
            dir.to_path_buf(),
            dir.join("Gauntlet"),
            dir.join("files"),
            dir.join("files").join("Gauntlet"),
        ];
        let data_root = candidates
            .iter()
            .find(|c| child_ci(c, "LEVELS").is_some_and(|l| l.is_dir()))
            .ok_or_else(|| InstallError::NoGameData(origin.to_path_buf()))?;

        let mut index = HashMap::new();
        let mut files = Vec::new();
        index_dir(data_root, "", &mut index, &mut files)?;
        files.sort();
        let levels = levels_in(&files);
        if levels.is_empty() {
            return Err(InstallError::NoGameData(origin.to_path_buf()));
        }

        // An extracted tree may carry the boot header near the data: beside
        // what the user pointed at, or one level up when they pointed
        // straight at the data folder (`.../Gauntlet` next to `.../sys`).
        let boot = [dir, data_root.parent().unwrap_or(dir)]
            .into_iter()
            .flat_map(|d| [d.join("sys").join("boot.bin"), d.join("boot.bin")])
            .find(|p| p.is_file())
            .and_then(|p| std::fs::File::open(p).ok())
            .and_then(|mut f| DiscHeader::read_from(&mut f).ok());
        let (game_id, title, warning) = match boot {
            Some(header) => {
                let (id, title, warning) = identify(&header);
                (Some(id), Some(title), warning)
            }
            None => (None, None, None),
        };

        Ok(Self {
            origin: origin.to_path_buf(),
            game_id,
            title,
            levels,
            warning,
            source: Source::Directory { index },
            files,
        })
    }

    /// Reads a game file by its path relative to the data root
    /// (`LEVELS/levelA1/objects.ngc`), case-insensitively.
    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, InstallError> {
        match &mut self.source {
            Source::Disc { disc, prefix } => match disc.read(&format!("{prefix}{path}")) {
                Err(FstError::NotFound(_)) => Err(InstallError::NotFound(path.to_string())),
                other => Ok(other?),
            },
            Source::Directory { index } => {
                let real = index
                    .get(&path.to_ascii_lowercase())
                    .ok_or_else(|| InstallError::NotFound(path.to_string()))?;
                Ok(std::fs::read(real)?)
            }
        }
    }

    pub fn exists(&self, path: &str) -> bool {
        let key = path.to_ascii_lowercase();
        match &self.source {
            Source::Disc { disc, prefix } => disc.fst.get(&format!("{prefix}{path}")).is_some(),
            Source::Directory { index } => index.contains_key(&key),
        }
    }

    /// Every game file, relative to the data root, original casing.
    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn source_kind(&self) -> &'static str {
        match self.source {
            Source::Disc { .. } => "disc image",
            Source::Directory { .. } => "extracted folder",
        }
    }
}

/// `"Gauntlet/LEVELS/levelA1/objects.ngc"` → `Some("Gauntlet/")`.
fn data_root_prefix(path: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    let idx = if lower.starts_with("levels/") {
        0
    } else {
        lower.find("/levels/")? + 1
    };
    lower.ends_with("/objects.ngc").then(|| path[..idx].to_string())
}

fn levels_in(files: &[String]) -> Vec<String> {
    let mut levels: Vec<String> = files
        .iter()
        .filter_map(|f| {
            let mut parts = f.split('/');
            let (Some(top), Some(level), Some(file), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return None;
            };
            (top.eq_ignore_ascii_case("LEVELS") && file.eq_ignore_ascii_case("objects.ngc"))
                .then(|| level.to_string())
        })
        .collect();
    levels.sort();
    levels.dedup();
    levels
}

fn identify(header: &DiscHeader) -> (String, String, Option<String>) {
    let id = header.game_id.clone();
    let warning = (!VERIFIED_GAME_IDS.contains(&id.as_str())).then(|| {
        format!(
            "game ID {id} hasn't been tested (verified: {}); loading anyway",
            VERIFIED_GAME_IDS.join(", ")
        )
    });
    (id, header.title.clone(), warning)
}

fn child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
    })
}

fn index_dir(
    dir: &Path,
    rel: &str,
    index: &mut HashMap<String, PathBuf>,
    files: &mut Vec<String>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let rel_path = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        let ty = entry.file_type()?;
        if ty.is_dir() {
            index_dir(&entry.path(), &rel_path, index, files)?;
        } else if ty.is_file() {
            index.insert(rel_path.to_ascii_lowercase(), entry.path());
            files.push(rel_path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISO: &str = "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet - Dark Legacy (USA).iso";
    const EXTRACTED: &str = "/Users/dmonskull/Desktop/GauntletDarkLegacy";

    fn real_iso() -> Option<PathBuf> {
        let p = PathBuf::from(std::env::var("GAUNTLET_DISC").unwrap_or_else(|_| ISO.into()));
        p.is_file().then_some(p)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gdl-install-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_data_root(root: &Path) {
        for level in ["levelA1", "levelB2"] {
            let d = root.join("LEVELS").join(level);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("objects.ngc"), level.as_bytes()).unwrap();
        }
    }

    #[test]
    fn finds_data_in_each_supported_folder_layout() {
        for (name, sub) in [
            ("data-root", ""),
            ("plain", "Gauntlet"),
            ("dolphin", "files"),
            ("dolphin-nested", "files/Gauntlet"),
        ] {
            let top = scratch(name);
            fake_data_root(&top.join(sub));
            let mut install = GameInstall::locate(&top).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(install.levels, ["levelA1", "levelB2"], "{name}");
            assert_eq!(install.read("levels/LEVELA1/OBJECTS.NGC").unwrap(), b"levelA1", "{name}");
            let _ = std::fs::remove_dir_all(&top);
        }
    }

    #[test]
    fn finds_data_from_a_dolphin_style_main_dol() {
        let top = scratch("dol");
        fake_data_root(&top.join("files"));
        std::fs::create_dir_all(top.join("sys")).unwrap();
        std::fs::write(top.join("sys").join("main.dol"), b"").unwrap();
        let install = GameInstall::locate(top.join("sys").join("main.dol")).unwrap();
        assert_eq!(install.levels.len(), 2);
        let _ = std::fs::remove_dir_all(&top);
    }

    #[test]
    fn explains_compressed_images_and_wrong_folders() {
        let top = scratch("errors");
        let rvz = top.join("game.rvz");
        std::fs::write(&rvz, b"RVZ\x01").unwrap();
        assert!(matches!(GameInstall::locate(&rvz), Err(InstallError::CompressedImage(..))));
        assert!(matches!(GameInstall::locate(&top), Err(InstallError::NoGameData(_))));
        assert!(matches!(
            GameInstall::locate(top.join("nope.iso")),
            Err(InstallError::Missing(_))
        ));
        let _ = std::fs::remove_dir_all(&top);
    }

    #[test]
    fn rejects_a_non_gamecube_file() {
        let top = scratch("junk");
        let junk = top.join("junk.iso");
        std::fs::write(&junk, vec![0u8; 0x2000]).unwrap();
        assert!(GameInstall::locate(&junk).is_err());
        let _ = std::fs::remove_dir_all(&top);
    }

    /// The real disc image and the user's own extracted copy must agree on
    /// identity, level list and file contents.
    #[test]
    fn real_disc_and_extracted_copy_agree() {
        let Some(iso) = real_iso() else {
            eprintln!("skipping: no disc image");
            return;
        };
        let mut disc = GameInstall::locate(&iso).unwrap();
        assert_eq!(disc.game_id.as_deref(), Some("GUNE5D"));
        assert!(disc.warning.is_none());
        assert!(disc.levels.len() > 60, "{} levels", disc.levels.len());
        eprintln!("{disc:?}");

        let extracted_dol = Path::new(EXTRACTED).join("sys").join("main.dol");
        if !extracted_dol.is_file() {
            return;
        }
        let mut tree = GameInstall::locate(&extracted_dol).unwrap();
        assert_eq!(tree.game_id.as_deref(), Some("GUNE5D"), "boot.bin next to main.dol");
        let data_folder = GameInstall::locate(Path::new(EXTRACTED).join("Gauntlet")).unwrap();
        assert_eq!(data_folder.game_id.as_deref(), Some("GUNE5D"), "boot.bin one level up");
        assert_eq!(disc.levels, tree.levels);
        for level in disc.levels.clone().iter().take(5) {
            let p = format!("LEVELS/{level}/objects.ngc");
            assert_eq!(disc.read(&p).unwrap(), tree.read(&p).unwrap(), "{p}");
        }
    }
}
