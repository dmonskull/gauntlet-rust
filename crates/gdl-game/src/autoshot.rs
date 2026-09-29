//! `GDL_SCREENSHOT=out.png` renders a few frames, saves a screenshot of the
//! primary window from inside the engine, then exits. For verifying visual
//! changes without a person looking at the window.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

const SHOOT_AT_FRAME: u32 = 30;
const EXIT_AT_FRAME: u32 = 60;

pub struct AutoShotPlugin;

#[derive(Resource)]
struct AutoShot {
    path: PathBuf,
    frame: u32,
}

impl Plugin for AutoShotPlugin {
    fn build(&self, app: &mut App) {
        if let Some(path) = std::env::var_os("GDL_SCREENSHOT") {
            app.insert_resource(AutoShot { path: path.into(), frame: 0 })
                .add_systems(Update, tick);
        }
    }
}

fn tick(mut commands: Commands, mut shot: ResMut<AutoShot>, mut exit: MessageWriter<AppExit>) {
    shot.frame += 1;
    if shot.frame == SHOOT_AT_FRAME {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(shot.path.clone()));
    }
    if shot.frame >= EXIT_AT_FRAME {
        exit.write(AppExit::Success);
    }
}
