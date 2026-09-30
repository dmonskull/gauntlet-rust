//! `GDL_SCREENSHOT=out.png` renders a few frames, saves a screenshot of the
//! primary window from inside the engine, then exits. For verifying visual
//! changes without a person looking at the window. `GDL_SHOT_AT=<frame>`
//! shoots later (default 30); `GDL_SHOTS=<n>` with `GDL_SHOT_EVERY=<k>`
//! takes n shots k frames apart (`out.png`, `out_1.png`, …).
//! `GDL_SHOT_CLOCK=ticks` counts both in 30 Hz game ticks instead of drawn
//! frames, so the shots land on the same moment of play however fast the
//! frames come (the game runs on its tick).

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

const SHOOT_AT_FRAME: u32 = 30;
/// Frames between the shot and exiting, for the file to be written.
const EXIT_AFTER: u32 = 30;

pub struct AutoShotPlugin;

#[derive(Resource)]
struct AutoShot {
    path: PathBuf,
    frame: u32,
    at: u32,
    count: u32,
    every: u32,
}

impl Plugin for AutoShotPlugin {
    fn build(&self, app: &mut App) {
        if let Some(path) = std::env::var_os("GDL_SCREENSHOT") {
            let at = std::env::var("GDL_SHOT_AT").ok().and_then(|v| v.parse().ok()).unwrap_or(SHOOT_AT_FRAME);
            let env = |k: &str, d: u32| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
            let (count, every) = (env("GDL_SHOTS", 1).max(1), env("GDL_SHOT_EVERY", 10).max(1));
            app.insert_resource(AutoShot { path: path.into(), frame: 0, at, count, every });
            if std::env::var("GDL_SHOT_CLOCK").is_ok_and(|v| v.eq_ignore_ascii_case("ticks")) {
                app.add_systems(FixedUpdate, tick);
            } else {
                app.add_systems(Update, tick);
            }
        }
    }
}

fn tick(mut commands: Commands, mut shot: ResMut<AutoShot>, mut exit: MessageWriter<AppExit>) {
    shot.frame += 1;
    let last = shot.at + (shot.count - 1) * shot.every;
    if shot.frame >= shot.at && shot.frame <= last && (shot.frame - shot.at).is_multiple_of(shot.every) {
        let k = (shot.frame - shot.at) / shot.every;
        let path = if k == 0 {
            shot.path.clone()
        } else {
            let stem = shot.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = shot.path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "png".into());
            shot.path.with_file_name(format!("{stem}_{k}.{ext}"))
        };
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
    if shot.frame >= last + EXIT_AFTER {
        exit.write(AppExit::Success);
    }
}
