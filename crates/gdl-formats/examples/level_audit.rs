//! Audits every level's population against the game's level build
//! (`docs/level-population.md`, "Auditing the levels"): what a one-player
//! game makes, the trigger links, and items that block the hero while
//! nothing is drawn (dev tool).
//!
//! ```text
//! cargo run -p gdl-formats --example level_audit -- <game>/Gauntlet [level] [--triggers] [--anim] [--falls]
//! ```
//!
//! With `--triggers`, every trigger the party has is listed too: where it
//! is (a warp point on the floor under it), what it is and what it moves.
//! With `--anim`, each animated object a trigger plays is described: the
//! middle of each of its nodes' collision at rest, at its first frame
//! (where the level starts it) and at its last (where the trigger takes
//! it). With `--falls`, the falling obstacles (rock falls, leaves, debris,
//! shot-down walls, sinking rocks): their model, shape, links and the
//! floor under each.
//!
//! Per level it lists:
//! - `MOVER`: world nodes the game moves (or hides) because a trigger for
//!   more players registers them — the game registers every trigger's
//!   target, made for the party or not — with the heights it holds them
//!   at;
//! - `LINK`: trigger links worth a look: chains to missing ids, quest
//!   gates, camera points, wake and shake flags, targets shared by
//!   triggers of different kinds;
//! - `PLACE`: placements the game builds differently: obelisks (never
//!   made), key rings, rotators without a model, random types;
//! - `BLOCK`: items that block a hero who walks into them but have no
//!   model the game's search finds (`placed without a model` when the
//!   placement says so: invisible in the original too);
//! - `ANIM`: trigger targets in the animated mode (under an animated
//!   object) without an animation of their own: the game neither moves nor
//!   plays them;
//! - `WALL`: secret walls (obstacle `0x2A`, shape 4), which block with
//!   their own collision triangles until they're shot.
//!
//! The model search is the game's (and `population.rs`'s): an atree of
//! the name in the realm's items, `POWERUPS` or the level's own items,
//! else an object named so, with `L1`, or with `L1ROOT`, there or in the
//! level's models. Generators aren't checked (their models come with the
//! realm's monsters).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use gdl_formats::anim::{AnimFile, ROTATION_BITS, TRANSLATION_BITS, rotation_matrix};
use gdl_formats::collision::{LevelCollision, NodePose};
use gdl_formats::population::{ItemClass, ItemType, LocatorKind, Placement, PlacementParams, Population};
use gdl_formats::{ModelFile, WorldFile};

/// Each node's parent.
fn parents(world: &WorldFile) -> Vec<Option<usize>> {
    let mut parent = vec![None; world.nodes.len()];
    for (i, n) in world.nodes.iter().enumerate() {
        let mut c = n.first_child;
        while let Some(k) = c {
            if k >= world.nodes.len() || parent[k].is_some() {
                break;
            }
            parent[k] = Some(i);
            c = world.nodes[k].next_sibling;
        }
    }
    parent
}

/// Whether `node` is `top` or under it.
fn under(parent: &[Option<usize>], node: usize, top: usize) -> bool {
    let mut n = Some(node);
    let mut steps = 0;
    while let Some(k) = n {
        if k == top {
            return true;
        }
        n = parent[k];
        steps += 1;
        if steps > parent.len() {
            break;
        }
    }
    false
}

/// Where an animated object's track puts its node at `frame`, about the
/// node's origin (as `mechanics.rs` poses it).
fn animated_pose(a: &gdl_formats::world::ObjectAnimation, frame: f32, origin: [f32; 3]) -> NodePose {
    let Some(track) = &a.track else { return NodePose::REST };
    let pose = track.sample(frame);
    let has = |bits: [u16; 3]| bits.iter().any(|&b| track.flags & b != 0);
    let rotation = if has(ROTATION_BITS) {
        let m = rotation_matrix(pose.rotation, track.flags);
        [m[0], m[4], m[8], m[1], m[5], m[9], m[2], m[6], m[10]]
    } else {
        NodePose::REST.rotation
    };
    let t = if has(TRANSLATION_BITS) { pose.translation } else { [0.0; 3] };
    let turned = NodePose { rotation, translation: [0.0; 3] }.apply_vector(origin);
    NodePose { rotation, translation: std::array::from_fn(|i| origin[i] + t[i] - turned[i]) }
}

/// A folder's object names and its atrees' parts.
#[derive(Default)]
struct Source {
    objects: HashSet<String>,
    /// Atree name → the object each node draws.
    atrees: HashMap<String, Vec<String>>,
}

impl Source {
    fn load(dir: &Path) -> Option<Self> {
        let model = ModelFile::parse(&std::fs::read(dir.join("objects.ngc")).ok()?).ok()?;
        let objects = model.objects.into_iter().map(|o| o.name).collect();
        let mut atrees = HashMap::new();
        if let Some(anim) = std::fs::read(dir.join("ANIM.PS2")).ok().and_then(|a| AnimFile::parse(&a).ok()) {
            for a in anim.atrees {
                let parts = (0..a.nodes.len())
                    .map(|i| match a.flipbook_entry(i, 0) {
                        Some(entry) => entry.first.clone(),
                        None => format!("{}{}", a.name, a.nodes[i].name),
                    })
                    .collect();
                atrees.insert(a.name.clone(), parts);
            }
        }
        Some(Self { objects, atrees })
    }
}

/// Whether the game finds a model for `name`.
fn has_model(name: &str, items: &[&Source], level: &Source) -> bool {
    if name.is_empty() {
        return false;
    }
    for s in items {
        if let Some(parts) = s.atrees.get(name)
            && parts.iter().any(|p| s.objects.contains(p))
        {
            return true;
        }
    }
    ["", "L1", "L1ROOT"].iter().any(|suffix| {
        let full = format!("{name}{suffix}");
        items.iter().any(|s| s.objects.contains(&full)) || level.objects.contains(&full)
    })
}

/// The trigger flags' low byte the game sets from the subtype.
fn trigger_flags(subtype: i32, placed: u16) -> u16 {
    let low = match subtype {
        0x14 => 0x10,
        0x15 => 0x08,
        0x16 => 0x12,
        0x17 => 0x0A,
        0x19 => 0x804,
        0x1A => 0x02,
        0x1B => 0x80C,
        0x1C => 0x09,
        0x1D => 0x0A,
        _ => return placed | 8,
    };
    (placed & 0xFF00) | low
}

fn shape(ty: &ItemType) -> u16 {
    u16::from_le_bytes([ty.raw[8], ty.raw[9]])
}

/// Whether a hero walking into a fresh item of this type is pushed back
/// (the touch handler's answer, before anything has happened to it).
fn blocks(ty: &ItemType, p: &Placement) -> bool {
    match ty.class {
        ItemClass::Container | ItemClass::Door | ItemClass::Generator => true,
        ItemClass::Obstacle => match ty.subtype {
            0x28 | 0x31 | 0x33..=0x35 => false,
            0x29 => matches!(p.params(ty.class), PlacementParams::Obstacle { count, .. } if count > 0),
            _ => true,
        },
        _ => false,
    }
}

const SUBTYPE_OBELISK: i32 = 12;
const SUBTYPE_KEY: i32 = 2;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let listing = args.iter().any(|a| a == "--triggers");
    let describe = args.iter().any(|a| a == "--anim");
    let falls = args.iter().any(|a| a == "--falls");
    let mut plain = args.iter().filter(|a| !a.starts_with("--"));
    let root = plain.next().expect("usage: level_audit <game>/Gauntlet [level] [--triggers] [--anim] [--falls]").clone();
    let only = plain.next().map(|s| s.to_ascii_lowercase());
    let root = Path::new(&root);
    let mut levels: Vec<_> = std::fs::read_dir(root.join("LEVELS")).expect("LEVELS").flatten().map(|e| e.path()).collect();
    levels.sort();
    let powerups = Source::load(&root.join("POWERUPS")).unwrap_or_default();
    let mut realm_items: BTreeMap<char, Source> = BTreeMap::new();
    let mut totals: BTreeMap<&str, usize> = BTreeMap::new();

    for dir in levels {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        if only.as_ref().is_some_and(|o| !name.eq_ignore_ascii_case(o)) {
            continue;
        }
        let Ok(bytes) = std::fs::read(dir.join("WORLDS.PS2")) else { continue };
        let (Ok(pop), Ok(world)) = (Population::parse(&bytes), WorldFile::parse(&bytes)) else { continue };
        let collision = LevelCollision::new(&world).ok();
        let floor = |at: [f32; 3]| collision.as_ref().and_then(|c| c.floor_height([at[0], at[1] + 2.0, at[2]]));
        let parent = parents(&world);
        let origins = world.world_positions().unwrap_or_default();
        // The loader puts each animated object's node, and every node
        // under it, in the animated mode.
        let in_mode = |node: usize| pop.animations.iter().any(|a| under(&parent, node, a.node));
        let animation_of = |node: usize| pop.animations.iter().find(|a| a.node == node);
        let letter = name.strip_prefix("level").and_then(|s| s.chars().next()).unwrap_or('A').to_ascii_uppercase();
        let realm = realm_items.entry(letter).or_insert_with(|| Source::load(&root.join(format!("ITEMS/level{letter}"))).unwrap_or_default());
        let own = Source::load(&root.join(format!("ITEMS/{name}"))).unwrap_or_default();
        let level = Source::load(&dir).unwrap_or_default();
        let items: Vec<&Source> = vec![&*realm, &powerups, &own];
        let mut lines: Vec<String> = Vec::new();
        let mut note = |kind: &'static str, text: String, lines: &mut Vec<String>| {
            *totals.entry(kind).or_default() += 1;
            lines.push(format!("  {kind:5} {text}"));
        };

        // Movers: the game registers each trigger's target as it makes
        // the item — made for the party or not — the first registration
        // setting its kind, flags and heights.
        struct Reg {
            placement: usize,
            subtype: i32,
            flags: u16,
            off: i16,
            on: i16,
            active: bool,
        }
        let mut registered: BTreeMap<usize, Vec<Reg>> = BTreeMap::new();
        let mut triggers = Vec::new();
        for (i, p) in pop.placements.iter().enumerate() {
            let ty = pop.resolved_type(p);
            if let PlacementParams::Trigger { target, flags, id, next, .. } = p.params(ty.class) {
                let flags = trigger_flags(ty.subtype, flags);
                triggers.push((i, ty.subtype, flags, id, next, p.active_for(1)));
                if let Some(t) = target.filter(|&t| t < world.nodes.len())
                    && let PlacementParams::Trigger { off, on, .. } = p.params(ty.class)
                {
                    registered.entry(t).or_default().push(Reg { placement: i, subtype: ty.subtype, flags, off, on, active: p.active_for(1) });
                }
            }
        }
        for (&node, regs) in &registered {
            let first = &regs[0];
            let kind = first.flags as u8;
            let what = if kind & 0x10 != 0 {
                if kind & 0x20 == 0 { "a bridge, hidden" } else { "a bridge, shown" }
            } else if first.off != 0 {
                "held at its off height"
            } else {
                "at rest"
            };
            if regs.iter().all(|r| !r.active) {
                note(
                    "MOVER",
                    format!(
                        "node {node} {} moved only by triggers for more players (placements {:?}, subtype {:#x}, flags {:#x}, off {:.1} on {:.1}): the game keeps it {what}",
                        world.nodes[node].name,
                        regs.iter().map(|r| r.placement).collect::<Vec<_>>(),
                        first.subtype,
                        first.flags,
                        0.1 * f32::from(first.off),
                        0.1 * f32::from(first.on),
                    ),
                    &mut lines,
                );
            } else if !first.active {
                let port = regs.iter().find(|r| r.active).unwrap();
                if (port.flags as u8, port.off, port.on) != (first.flags as u8, first.off, first.on) {
                    note(
                        "MOVER",
                        format!(
                            "node {node} {}: first registered by placement {} for more players (flags {:#x}, off {:.1} on {:.1}), the party's trigger {} has flags {:#x}, off {:.1} on {:.1}",
                            world.nodes[node].name,
                            first.placement,
                            first.flags,
                            0.1 * f32::from(first.off),
                            0.1 * f32::from(first.on),
                            port.placement,
                            port.flags,
                            0.1 * f32::from(port.off),
                            0.1 * f32::from(port.on),
                        ),
                        &mut lines,
                    );
                }
            }
            let kinds: HashSet<i32> = regs.iter().map(|r| if (0x1B..=0x1D).contains(&r.subtype) { 0x1B } else { r.subtype }).collect();
            if kinds.len() > 1 {
                note("LINK", format!("node {node} {} is moved by triggers of different kinds {kinds:?}", world.nodes[node].name), &mut lines);
            }
        }

        // Trigger links.
        let ids: HashMap<u8, usize> = triggers.iter().filter(|t| t.5 && t.3 != 0).map(|t| (t.3, t.0)).collect();
        for &(i, subtype, flags, id, next, active) in &triggers {
            if !active {
                continue;
            }
            if next != 0 && !ids.contains_key(&next) {
                note("LINK", format!("trigger {i} (subtype {subtype:#x}) chains to id {next}, which no trigger for one player has"), &mut lines);
            }
            if flags & 0x40 != 0 && !name.eq_ignore_ascii_case("levelL1") && !name.eq_ignore_ascii_case("levelL3") {
                note("LINK", format!("trigger {i} (subtype {subtype:#x}, id {id}) is a quest gate outside the tower"), &mut lines);
            }
            let camera = id != 0 && pop.locators.iter().any(|l| l.kind == LocatorKind::Transmitter(9) && l.index == i16::from(id));
            let odd = flags & !(0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x20 | 0x40 | 0x80 | 0x100 | 0x200 | 0x400 | 0x800 | 0x1000 | 0x2000);
            if odd != 0 {
                note("LINK", format!("trigger {i} (subtype {subtype:#x}) has flags {odd:#x} nothing decodes (all {flags:#x}){}", if camera { ", with a camera point" } else { "" }), &mut lines);
            }
        }

        // Trigger targets in the animated mode with no animation of their
        // own: nothing moves them.
        for &node in registered.keys() {
            if in_mode(node) && animation_of(node).is_none_or(|a| a.track.is_none()) {
                note("ANIM", format!("trigger target node {node} {} is under an animated object but has no animation: nothing moves it", world.nodes[node].name), &mut lines);
            }
        }
        // What the triggers' animated objects do.
        if describe {
            for &node in registered.keys() {
                let Some(a) = animation_of(node).filter(|a| a.track.is_some()) else { continue };
                let origin = origins.get(node).copied().flatten().unwrap_or_default();
                let (first, last) = (animated_pose(a, 0.0, origin), animated_pose(a, f32::from(a.frames - 1), origin));
                lines.push(format!("  ANIMD node {node} {} ({} frames, {:.1} s) at ({:.1}, {:.1}, {:.1}):", world.nodes[node].name, a.frames, f32::from(a.frames) / 30.0, origin[0], origin[1], origin[2]));
                let Some(c) = &collision else { continue };
                let mut parts: BTreeMap<usize, Vec<[f32; 3]>> = BTreeMap::new();
                for (n, _, corners) in c.world_triangles() {
                    if under(&parent, n, node) {
                        parts.entry(n).or_default().extend(corners);
                    }
                }
                for (n, points) in parts {
                    let mid = |pose: &NodePose| {
                        let sum = points.iter().fold([0.0f32; 3], |s, &p| {
                            let q = pose.apply(p);
                            [s[0] + q[0], s[1] + q[1], s[2] + q[2]]
                        });
                        sum.map(|v| v / points.len() as f32)
                    };
                    let (r, f, l) = (mid(&NodePose::REST), mid(&first), mid(&last));
                    let moves = if world.nodes[n].flags & (0x1000 | 0x100_0000) != 0 { "its collision follows" } else { "collision stays" };
                    lines.push(format!(
                        "        {:16} rest ({:.1}, {:.1}, {:.1}) first ({:.1}, {:.1}, {:.1}) last ({:.1}, {:.1}, {:.1}); {moves}",
                        world.nodes[n].name, r[0], r[1], r[2], f[0], f[1], f[2], l[0], l[1], l[2]
                    ));
                }
            }
        }
        let walls: Vec<usize> = pop
            .placements
            .iter()
            .enumerate()
            .filter(|(_, p)| p.active_for(1) && { let t = pop.resolved_type(p); t.class == ItemClass::Obstacle && t.subtype == 0x2A })
            .map(|(i, _)| i)
            .collect();
        if !walls.is_empty() {
            note("WALL", format!("{} secret walls (placements {walls:?})", walls.len()), &mut lines);
        }

        // Crumbling floors: what they are and what's under them.
        if falls {
            for (i, p) in pop.placements.iter().enumerate() {
                let ty = pop.resolved_type(p);
                if !p.active_for(1) || ty.class != ItemClass::Obstacle || !matches!(ty.subtype, 0x28 | 0x31 | 0x33..=0x35) {
                    continue;
                }
                let f = |o: usize| f32::from_le_bytes(ty.raw[o..o + 4].try_into().unwrap());
                let extents = [f(0x0C), f(0x10), f(0x14), f(0x18)];
                let under = collision.as_ref().and_then(|c| c.floor_probe(p.position, 4.0, -10.0, 1.0, 0));
                let floor = under.map_or("no floor".to_string(), |h| {
                    let n = &world.nodes[h.node];
                    format!("floor {:.2} on node {} {} (flags {:#x})", h.point[1], h.node, n.name, n.flags)
                });
                let model = if has_model(p.model_name(ty), &items, &level) { "" } else { " (no model found)" };
                lines.push(format!(
                    "  FALL  {i:4} {:#x} {} model {}{model} shape {} extents {extents:?} links {:?} rot {:?} at ({:.1}, {:.1}, {:.1}); {floor}",
                    ty.subtype,
                    ty.name,
                    p.model_name(ty),
                    shape(ty),
                    p.links,
                    p.rotation,
                    p.position[0],
                    p.position[1],
                    p.position[2]
                ));
            }
        }

        // The party's triggers, for checking them in the game.
        if listing {
            for &(i, subtype, flags, id, next, active) in &triggers {
                if !active {
                    continue;
                }
                let p = &pop.placements[i];
                let PlacementParams::Trigger { target, sound, .. } = p.params(pop.resolved_type(p).class) else { continue };
                let kind = match subtype {
                    0x14 => "BRIDGEPAD",
                    0x15 => "DOORPAD",
                    0x16 => "BRIDGESW",
                    0x17 => "DOORSW",
                    0x19 => "ELEVPAD",
                    0x1A => "ELEVSW",
                    0x1B => "LIFTPAD",
                    0x1C => "LIFTSW",
                    0x1D => "LIFTEND",
                    0x1F => "hit switch",
                    _ => "ACTIVESW",
                };
                let what = if flags & 1 != 0 {
                    "turns its target off"
                } else if flags & 2 != 0 {
                    "turns its target on for good"
                } else if flags & 4 != 0 {
                    "sends a lift while held"
                } else {
                    "holds its target on while stood on"
                };
                let moves = match target.filter(|&t| t < world.nodes.len()) {
                    Some(t) => {
                        let mover = &registered[&t][0];
                        let k = mover.flags as u8;
                        if k & 0x10 == 0 && in_mode(t) {
                            match animation_of(t).filter(|a| a.track.is_some()) {
                                Some(a) => format!("plays node {t} {}'s animation ({} frames) on, back while off", world.nodes[t].name, a.frames),
                                None => format!("node {t} {} (under an animated object: nothing)", world.nodes[t].name),
                            }
                        } else if k & 0x10 != 0 {
                            format!("node {t} {} (a bridge: {})", world.nodes[t].name, if k & 0x20 == 0 { "shown while on" } else { "hidden while on" })
                        } else {
                            format!("node {t} {} from {:.1} to {:.1}", world.nodes[t].name, 0.1 * f32::from(mover.off), 0.1 * f32::from(mover.on))
                        }
                    }
                    None => "nothing (its own animation)".into(),
                };
                let at = p.position;
                let y = floor(at).map_or("no floor".to_string(), |y| format!("{y:.2}"));
                let chain = if next != 0 { format!(", then id {next}") } else { String::new() };
                let extra = [(0x40, " quest gate"), (0x100, " stand on target"), (0x1000, " shakes"), (0x2000, " wakes a statue")]
                    .iter()
                    .filter(|(b, _)| flags & b != 0)
                    .map(|(_, t)| *t)
                    .collect::<String>();
                lines.push(format!(
                    "  TRIG  {i:4} {kind:10} id {id:3}{chain}: {what}: {moves}{extra}; sound {sound}; at ({:.1}, {:.1}, {:.1}) floor {y}",
                    at[0], at[1], at[2]
                ));
            }
        }

        // Placements.
        for (i, p) in pop.placements.iter().enumerate() {
            if !p.active_for(1) {
                continue;
            }
            let raw = &pop.item_types[p.item_type];
            if raw.class == ItemClass::Random {
                let choices: Vec<&str> = raw.choices.iter().map(|&c| pop.resolve(c).name.as_str()).collect();
                note("PLACE", format!("placement {i} is a random type of {choices:?}: the game picks one each time the level is built"), &mut lines);
            }
            let ty = pop.resolved_type(p);
            match ty.class {
                ItemClass::Powerup if ty.subtype == SUBTYPE_OBELISK => {
                    note("PLACE", format!("placement {i} is an obelisk ({}): the game never makes it", ty.name), &mut lines);
                }
                ItemClass::Powerup if ty.subtype == SUBTYPE_KEY => {
                    if let PlacementParams::Powerup { count } = p.params(ty.class)
                        && count > 1
                    {
                        note("PLACE", format!("placement {i}: {count} keys, the game's KEYRING type"), &mut lines);
                    }
                }
                ItemClass::Rotator if ty.subtype != 2 && has_model(p.model_name(ty), &items, &level) => {
                    note("PLACE", format!("placement {i}: rotator subtype {} has a model ({}) the game doesn't draw", ty.subtype, ty.name), &mut lines);
                }
                _ => {}
            }
            // Blockers.
            if !blocks(ty, p) || ty.class == ItemClass::Generator || !matches!(shape(ty), 1..=3) {
                continue;
            }
            let model = match p.params(ty.class) {
                // A safe rock shows its stage (`SAFEROCK3`).
                PlacementParams::Obstacle { count, .. } if ty.subtype == 0x29 => format!("{}{count}", ty.name),
                _ => p.model_name(ty).to_string(),
            };
            let drawn = p.flags & 2 == 0 && has_model(&model, &items, &level);
            if !drawn {
                let why = if p.flags & 2 != 0 { "placed without a model" } else { "no model found" };
                note(
                    "BLOCK",
                    format!(
                        "placement {i} {:?} {:#x} {model:?} ({why}), shape {} radius {:.1} at ({:.1}, {:.1}, {:.1})",
                        ty.class,
                        ty.subtype,
                        shape(ty),
                        ty.extent[0],
                        p.position[0],
                        p.position[1],
                        p.position[2]
                    ),
                    &mut lines,
                );
            }
        }
        if !lines.is_empty() {
            println!("{name}:");
            for l in lines {
                println!("{l}");
            }
        }
    }
    println!("totals: {totals:?}");
}
