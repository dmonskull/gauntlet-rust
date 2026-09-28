//! Entry point. Points at your own legally-owned Gauntlet: Dark Legacy
//! (GameCube) disc image via the `GAUNTLET_DISC` environment variable, reads
//! its boot header and main.dol layout, then opens a bare window as a
//! plumbing check. No game assets ship in this repository.

use std::fs::File;
use std::io::BufReader;

use bevy::prelude::*;
use gdl_formats::{DiscHeader, DolHeader};

fn main() {
    let disc_info = match load_disc_info() {
        Ok(info) => info,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    println!("{disc_info}");

    App::new()
        .insert_resource(WindowTitle(disc_info))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "gdl-game".into(),
                ..default()
            }),
            ..default()
        }))
        .add_systems(Startup, set_window_title)
        .run();
}

#[derive(Resource)]
struct WindowTitle(String);

fn set_window_title(title: Res<WindowTitle>, mut windows: Query<&mut Window>) {
    if let Ok(mut window) = windows.single_mut() {
        window.title = format!("gdl-game — {}", title.0);
    }
}

fn load_disc_info() -> Result<String, String> {
    let disc_path = std::env::var("GAUNTLET_DISC").map_err(|_| {
        "Set GAUNTLET_DISC to the path of your own Gauntlet: Dark Legacy (GameCube) \
         disc image (.iso/.gcm) before running gdl-game."
            .to_string()
    })?;

    let file = File::open(&disc_path)
        .map_err(|e| format!("Failed to open '{disc_path}': {e}"))?;
    let mut reader = BufReader::new(file);

    let boot = DiscHeader::read_from(&mut reader)
        .map_err(|e| format!("Failed to read disc header from '{disc_path}': {e}"))?;
    let dol = DolHeader::read_from(&mut reader, boot.dol_offset)
        .map_err(|e| format!("Failed to read main.dol header: {e}"))?;

    Ok(format!(
        "{} ({}) — main.dol @ 0x{:X}, {} bytes, entry 0x{:08X}",
        boot.title,
        boot.game_id,
        boot.dol_offset,
        dol.file_size(),
        dol.entry_point,
    ))
}
