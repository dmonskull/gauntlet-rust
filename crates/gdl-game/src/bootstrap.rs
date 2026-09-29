//! Finds the user's game before the engine starts: command-line path, then
//! `GAUNTLET_GAME`, then the path remembered from last launch, then a native
//! file picker. The chosen path is remembered in `gdl-artifacts/` next to the
//! executable — the game's own files are never written to.

use std::path::{Path, PathBuf};

use gdl_install::GameInstall;

const USAGE: &str = "\
usage: gdl-game [GAME] [--level NAME] [--forget]
       gdl-game [GAME] --viewer [--character CLASS] [--variant V] [--action NAME]

  GAME             your Gauntlet: Dark Legacy disc image (.iso/.gcm), extracted
                   disc folder, or its main.dol. Remembered for next time.
  --level NAME     level to load (e.g. levelA1). Defaults to the first level.
  --forget         forget the remembered game path and ask again.
  --viewer         character viewer instead of the levels.
  --character CLS  player class to show (e.g. ARC, KNI, WIZ).
  --variant V      colour/armour folder (default BLU; e.g. RED, YEL40).
  --action NAME    action to start with (e.g. RUN1).

  env: GAUNTLET_GAME (same as GAME), GDL_ARTIFACTS (settings folder)";

pub struct Args {
    pub game: Option<PathBuf>,
    pub level: Option<String>,
    pub forget: bool,
    pub viewer: bool,
    pub character: Option<String>,
    pub variant: Option<String>,
    pub action: Option<String>,
}

impl Args {
    pub fn parse() -> Result<Self, String> {
        let mut args = Self {
            game: None,
            level: None,
            forget: false,
            viewer: false,
            character: None,
            variant: None,
            action: None,
        };
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "-h" | "--help" => return Err(USAGE.to_string()),
                "--forget" => args.forget = true,
                "--level" => args.level = Some(it.next().ok_or("--level needs a level name")?),
                "--viewer" => args.viewer = true,
                "--character" => args.character = Some(it.next().ok_or("--character needs a class")?),
                "--variant" => args.variant = Some(it.next().ok_or("--variant needs a folder name")?),
                "--action" => args.action = Some(it.next().ok_or("--action needs an action name")?),
                flag if flag.starts_with('-') => return Err(format!("unknown option {flag}\n\n{USAGE}")),
                path if args.game.is_none() => args.game = Some(PathBuf::from(path)),
                extra => return Err(format!("unexpected argument {extra}\n\n{USAGE}")),
            }
        }
        Ok(args)
    }
}

/// `gdl-artifacts/` beside the executable (like iw4l's `iw4l-artifacts/`),
/// overridable with `GDL_ARTIFACTS`.
pub fn artifacts_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("GDL_ARTIFACTS") {
        return PathBuf::from(dir);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gdl-artifacts")
}

struct Settings {
    file: PathBuf,
    game: Option<PathBuf>,
}

impl Settings {
    fn load() -> Self {
        let file = artifacts_dir().join("settings.txt");
        let game = std::fs::read_to_string(&file).ok().and_then(|text| {
            text.lines()
                .find_map(|l| l.strip_prefix("game="))
                .map(|p| PathBuf::from(p.trim()))
        });
        Self { file, game }
    }

    fn save(&self) {
        let body = match &self.game {
            Some(p) => format!("game={}\n", p.display()),
            None => String::new(),
        };
        let result = self
            .file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&self.file, body));
        if let Err(e) = result {
            eprintln!("warning: couldn't save settings to {}: {e}", self.file.display());
        }
    }
}

/// Resolves and validates the game install, remembering it on success.
pub fn resolve(args: &Args) -> Result<GameInstall, String> {
    let mut settings = Settings::load();
    if args.forget {
        settings.game = None;
        settings.save();
    }

    let explicit = args
        .game
        .clone()
        .or_else(|| std::env::var_os("GAUNTLET_GAME").map(PathBuf::from))
        .or_else(|| std::env::var_os("GAUNTLET_DISC").map(PathBuf::from));
    if let Some(path) = explicit {
        // An explicit path that doesn't work is an error, not a reason to
        // silently fall back to something else.
        let install = GameInstall::locate(&path).map_err(|e| e.to_string())?;
        remember(&mut settings, &install);
        return Ok(install);
    }

    if let Some(saved) = settings.game.clone() {
        match GameInstall::locate(&saved) {
            Ok(install) => return Ok(install),
            Err(e) => eprintln!("The remembered game path no longer works ({e}); asking again."),
        }
    }

    let mut problem: Option<String> = None;
    loop {
        if let Some(why) = problem.take() {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("That isn't Gauntlet: Dark Legacy")
                .set_description(&why)
                .show();
        }
        let picked = rfd::FileDialog::new()
            .set_title("Select your Gauntlet: Dark Legacy (GameCube) disc image or main.dol")
            .add_filter("GameCube disc image or main.dol", &["iso", "gcm", "dol", "rvz"])
            .add_filter("All files", &["*"])
            .pick_file()
            .ok_or_else(|| {
                format!("No game selected. You can also pass it on the command line.\n\n{USAGE}")
            })?;
        match GameInstall::locate(&picked) {
            Ok(install) => {
                remember(&mut settings, &install);
                return Ok(install);
            }
            Err(e) => problem = Some(e.to_string()),
        }
    }
}

fn remember(settings: &mut Settings, install: &GameInstall) {
    let origin = std::fs::canonicalize(&install.origin).unwrap_or_else(|_| install.origin.clone());
    if settings.game.as_ref() != Some(&origin) {
        settings.game = Some(origin);
        settings.save();
    }
}
