//! Gauntlet: Dark Legacy runtime. Point it at your own copy of the game —
//! disc image, extracted folder or main.dol — and it finds, validates and
//! loads the rest. No game assets ship with this program.

mod actions;
mod audio;
mod autoshot;
mod billboard;
mod boss_camera;
mod breakables;
mod bootstrap;
mod camera;
mod camera_rig;
mod character;
mod collision_debug;
mod combat;
mod controls;
mod critters;
mod damage;
mod deaths;
mod effects;
mod exits;
mod fade;
mod fake_pad;
mod familiars;
mod flash;
mod first_person;
mod font;
mod footsteps;
mod frame_rate;
mod frontend;
mod game_hud;
mod gamma;
mod going_out;
mod generators;
mod hazards;
mod hints;
mod hud;
mod items;
mod level;
mod level_intro;
mod level_material;
mod levelup;
mod locomotion;
mod loot;
mod mechanics;
mod message_box;
mod model_mesh;
mod options;
mod monsters;
mod online;
mod particles;
mod party;
mod pickup_notices;
mod play_camera;
mod player;
mod player_state;
mod power_looks;
mod power_menu;
mod texanim;
mod tower;
mod tower_scenes;
mod population;
mod quest;
mod rumble;
mod saves;
mod scene_light;
mod shop;
mod projectiles;
mod status_hud;
mod viewer;
mod world;

use bevy::prelude::*;

use bootstrap::Args;
use level::LoadedGame;

/// With `GDL_FPS`, names every frame over 20 ms (hitches).
/// Whether the developer keys work (`GDL_DEV_KEYS=1`, or the settings'
/// Debug page): `I` cycles the
/// population view, `K` the collision overlay, `C` the free camera, `F1`
/// the debug readouts, `M` mutes, `N` plays the bank's next sound, `[`/`]`
/// (Page Up/Down) change level. The original has none of these, and in
/// play they sit among the keyboard controls (`I` between magic and
/// strafe), so they're off unless asked for.
pub(crate) fn dev_keys() -> bool {
    options::dev_keys_on()
}

fn log_slow_frames(time: Res<Time<Real>>, mut frame: Local<u64>) {
    *frame += 1;
    let dt = time.delta_secs() * 1000.0;
    if *frame > 60 && dt > 20.0 {
        info!("slow frame {}: {dt:.1} ms", *frame);
    }
}

fn main() {
    let args = match Args::parse() {
        Ok(args) => args,
        Err(message) => {
            println!("{message}");
            return;
        }
    };

    let install = match bootstrap::resolve(&args) {
        Ok(install) => install,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    println!(
        "Game: {} ({}) from {} {}",
        install.title.as_deref().unwrap_or("Gauntlet: Dark Legacy"),
        install.game_id.as_deref().unwrap_or("id unknown"),
        install.source_kind(),
        install.origin.display()
    );
    if let Some(warning) = &install.warning {
        eprintln!("warning: {warning}");
    }

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window { title: "Gauntlet: Dark Legacy".into(), ..default() }),
        ..default()
    }))
    .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.03)))
    .add_plugins((
        level_material::LevelMaterialPlugin,
        gamma::GammaBlendPlugin,
        camera::CameraPlugin,
        character::CharacterPlugin,
        billboard::BillboardPlugin,
        options::OptionsPlugin,
        texanim::TexAnimPlugin,
        particles::ParticlesPlugin,
        autoshot::AutoShotPlugin,
    ));

    // The frame rate, for the settings' readout; GDL_FPS=1 also logs it
    // and its frame time every second.
    app.add_plugins((bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(), frame_rate::FrameRatePlugin));
    if std::env::var("GDL_FPS").is_ok_and(|v| !v.is_empty() && v != "0") {
        app.add_plugins(bevy::diagnostic::LogDiagnosticsPlugin::default()).add_systems(Last, log_slow_frames);
    }
    // GDL_MEMSTATS=1 logs what's alive every 2 s (entities and assets), to
    // find anything a level change leaves behind.
    if std::env::var("GDL_MEMSTATS").is_ok_and(|v| !v.is_empty() && v != "0") {
        app.add_systems(Last, log_memstats);
    }

    if args.viewer {
        let viewer = viewer::Viewer::new(
            install,
            args.character.as_deref(),
            args.monster.as_deref(),
            args.variant.as_deref(),
            args.action.as_deref(),
        );
        match viewer {
            Ok(v) => app.insert_resource(v).add_plugins(viewer::ViewerPlugin),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        };
    } else {
        // The front end runs unless the command line picked a level or a
        // hero; a new game starts in the tower hub.
        let skip_menus = args.level.is_some() || args.character.is_some();
        let first_level = args.level.as_deref().or((!skip_menus).then_some(frontend::TOWER));
        let mut game = match LoadedGame::load(install, first_level) {
            Ok(game) => game,
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        };
        println!("{}; starting in {}", game.summary_line(), game.current_name());
        for (name, why) in &game.failures {
            eprintln!("warning: level {name} failed to load: {why}");
        }
        // A hero picked on the command line plays at once as player 1 (the
        // front end's select screen fills the party otherwise).
        let mut party = party::Party::default();
        if skip_menus {
            let choice = player::PlayerChoice {
                class: args.character.as_deref().unwrap_or("WAR").to_ascii_uppercase(),
                variant: args.variant.as_deref().unwrap_or("BLU").to_ascii_uppercase(),
            };
            let devices = party::Devices { keyboard: true, ..default() };
            party.join(0, player_state::new_member(&mut game.install, choice, "LARRY", None, devices));
        }
        app.insert_resource(game)
            .insert_resource(party)
            .add_plugins((world::WorldPlugin, hud::HudPlugin, player::PlayerPlugin, combat::CombatPlugin, damage::DamagePlugin, play_camera::PlayCameraPlugin, first_person::FirstPersonPlugin, audio::GameAudioPlugin, population::PopulationPlugin, collision_debug::CollisionDebugPlugin))
            .add_plugins(level_intro::LevelIntroPlugin)
            .add_plugins((monsters::MonstersPlugin, projectiles::ProjectilesPlugin, critters::CrittersPlugin, effects::EffectsPlugin, deaths::DeathsPlugin, flash::FlashPlugin, fade::FadePlugin, quest::QuestPlugin, scene_light::SceneLightPlugin, saves::SavesPlugin))
            .add_plugins((
                player_state::PlayerStatePlugin,
                items::ItemsPlugin,
                mechanics::MechanicsPlugin,
                hazards::HazardsPlugin,
                breakables::BreakablesPlugin,
                exits::ExitsPlugin,
                hints::HintsPlugin,
                status_hud::StatusHudPlugin,
                power_looks::PowerLooksPlugin,
                familiars::FamiliarsPlugin,
                levelup::LevelUpPlugin,
                footsteps::FootstepsPlugin,
                loot::LootPlugin,
                power_menu::PowerMenuPlugin,
                rumble::RumblePlugin,
            ))
            .add_plugins((party::PartyPlugin, fake_pad::FakePadPlugin, online::OnlinePlugin))
            .add_plugins((
                font::Screen2dPlugin,
                frontend::FrontendPlugin { skip: skip_menus },
                game_hud::GameHudPlugin,
                tower::TowerPlugin,
                tower_scenes::TowerScenesPlugin,
                message_box::MessageBoxPlugin,
                pickup_notices::PickupNoticesPlugin,
                shop::ShopPlugin,
            ));
    }
    app.run();
}

/// `GDL_MEMSTATS`: counts of entities (all, and by a few markers) and of
/// the asset kinds levels make.
#[allow(clippy::too_many_arguments)]
fn log_memstats(
    time: Res<Time<Real>>,
    mut next: Local<f32>,
    entities: Query<Entity>,
    level: Query<(), With<world::LevelEntity>>,
    meshed: Query<(), With<Mesh3d>>,
    meshes: Res<Assets<Mesh>>,
    level_materials: Res<Assets<level_material::LevelMaterial>>,
    standard: Res<Assets<StandardMaterial>>,
    images: Res<Assets<Image>>,
    audio: Res<Assets<bevy::audio::AudioSource>>,
) {
    if time.elapsed_secs() < *next {
        return;
    }
    *next = time.elapsed_secs() + 2.0;
    info!(
        "memstats: {} entities ({} level, {} meshed), {} meshes, {} level materials, {} standard materials, {} images, {} audio sources",
        entities.iter().count(),
        level.iter().count(),
        meshed.iter().count(),
        meshes.len(),
        level_materials.len(),
        standard.len(),
        images.len(),
        audio.len()
    );
}
