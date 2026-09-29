//! `GDL_SCREENSHOT=out.png` renders a few frames, saves a screenshot of the
//! primary window from inside the engine, then exits. For verifying visual
//! changes without a person looking at the window. `GDL_SHOT_AT=<frame>`
//! shoots later (default 30).

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
}

impl Plugin for AutoShotPlugin {
    fn build(&self, app: &mut App) {
        if let Some(path) = std::env::var_os("GDL_SCREENSHOT") {
            let at = std::env::var("GDL_SHOT_AT").ok().and_then(|v| v.parse().ok()).unwrap_or(SHOOT_AT_FRAME);
            app.insert_resource(AutoShot { path: path.into(), frame: 0, at })
                .add_systems(Update, tick);
        }
    }
}

fn tick(mut commands: Commands, mut shot: ResMut<AutoShot>, mut exit: MessageWriter<AppExit>) {
    shot.frame += 1;
    if shot.frame == shot.at {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(shot.path.clone()));
    }
    if shot.frame >= shot.at + EXIT_AFTER {
        exit.write(AppExit::Success);
    }
}
