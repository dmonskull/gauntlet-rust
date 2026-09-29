//! The game's 2D screen and its bitmap text (`docs/frontend.md`).
//!
//! The game draws its 2D layer — menus, title art, text — in a 512 × 384
//! screen, redrawn every frame from lists of blits and text strings. This
//! does the same: systems push [`Quad`]s into [`Draw2d`] during `Update`,
//! and a pool of Bevy UI image nodes shows them, letterboxed to 4:3 over
//! whatever the 3D camera draws.
//!
//! Text uses the game's own fonts (`FONTS/*.FNT` metrics with the texture
//! of the same name in `STATIC/`; `gdl_formats::font`). A font whose file or
//! texture can't be read falls back to Bevy's built-in font (ASCII only),
//! drawn at the same size.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::UiSystems;
use gdl_formats::ModelFile;
use gdl_formats::font::{FONT_SLOTS, FontFile, Placed};
use gdl_formats::texture;
use gdl_install::GameInstall;

use crate::level::LoadedGame;

/// The game's 2D screen.
pub const SCREEN_W: f32 = 512.0;
pub const SCREEN_H: f32 = 384.0;
/// Glow text (the game's draw flag `0x4000`) grows each glyph quad by this
/// much on every side (the game's 2.0 in screen pixels, about 2 here).
const GLOW_MARGIN: f32 = 2.0;

pub struct Screen2dPlugin;

impl Plugin for Screen2dPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Draw2d>()
            .add_systems(Startup, (load_ui, spawn_root).chain())
            .add_systems(First, |mut d: ResMut<Draw2d>| d.quads.clear())
            .add_systems(PostUpdate, flush.before(UiSystems::Prepare));
    }
}

/// A decoded texture for the 2D layer.
#[derive(Clone, Debug)]
pub struct UiImage {
    pub handle: Handle<Image>,
    pub size: Vec2,
}

/// Textures of the game's 2D model files, found by name (`TITLE00`,
/// `SCROLL_A`…) and decoded on first use.
#[derive(Resource)]
pub struct UiTextures {
    dirs: Vec<(ModelFile, Vec<u8>)>,
    names: HashMap<String, (usize, u16)>,
    cache: HashMap<(usize, u16), Option<UiImage>>,
}

impl UiTextures {
    /// Reads each folder's `objects.ngc` + `textures.ngc`; earlier folders
    /// win a name clash.
    pub fn load(install: &mut GameInstall, dirs: &[&str]) -> Self {
        let mut out = Self { dirs: Vec::new(), names: HashMap::new(), cache: HashMap::new() };
        for dir in dirs {
            let files = install
                .read(&format!("{dir}/objects.ngc"))
                .map_err(|e| e.to_string())
                .and_then(|o| ModelFile::parse(&o).map_err(|e| e.to_string()))
                .and_then(|m| Ok((m, install.read(&format!("{dir}/textures.ngc")).map_err(|e| e.to_string())?)));
            match files {
                Ok((model, textures)) => {
                    let index = out.dirs.len();
                    for t in &model.texture_names {
                        out.names.entry(t.name.to_ascii_uppercase()).or_insert((index, t.binding));
                    }
                    out.dirs.push((model, textures));
                }
                Err(why) => warn!("2D textures in {dir}: {why}"),
            }
        }
        out
    }

    pub fn get(&mut self, name: &str, images: &mut Assets<Image>) -> Option<UiImage> {
        self.frame(name, 0, images)
    }

    /// The texture `offset` bindings after the named one: the game's
    /// flipbooks (`GLOWCROP_00`, `LOGO_BURN1`…) step through bindings.
    pub fn frame(&mut self, name: &str, offset: u16, images: &mut Assets<Image>) -> Option<UiImage> {
        let &(dir, binding) = self.names.get(&name.to_ascii_uppercase())?;
        let key = (dir, binding + offset);
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }
        let (model, textures) = &self.dirs[dir];
        let image = model
            .bindings
            .get(key.1 as usize)
            .filter(|b| b.is_textured())
            .and_then(|b| texture::decode(textures, b).ok())
            .map(|rgba| UiImage { size: Vec2::new(rgba.width as f32, rgba.height as f32), handle: images.add(to_ui_image(rgba)) });
        self.cache.insert(key, image.clone());
        image
    }
}

fn to_ui_image(image: texture::RgbaImage) -> Image {
    let mut out = Image::new(
        Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        image.pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    out.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        ..ImageSamplerDescriptor::linear()
    });
    out
}

/// One of the game's fonts, ready to draw.
pub struct LoadedFont {
    pub file: FontFile,
    pub image: UiImage,
    pub space_width: u32,
}

/// The game's font slots, and the alternative textures some text is drawn
/// with (same glyph layout as `FONT32`, other images).
#[derive(Resource)]
pub struct GameFonts {
    pub slots: Vec<Option<LoadedFont>>,
    pub alternates: HashMap<&'static str, UiImage>,
}

/// Texture a line of `FONT32` text is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FontTexture {
    #[default]
    Own,
    /// `FONT32_PARCH`: ink-on-parchment letters (the in-game menus).
    Parch,
    /// `FONT32_GLOW`: blurred letters for the glow pass.
    Glow,
    /// `FONT32GAR0`..`5`: the frames menu letters flicker through as a
    /// menu opens.
    Gar(u8),
}

const ALTERNATES: [&str; 8] =
    ["FONT32_PARCH", "FONT32_GLOW", "FONT32GAR0", "FONT32GAR1", "FONT32GAR2", "FONT32GAR3", "FONT32GAR4", "FONT32GAR5"];

impl GameFonts {
    fn load(install: &mut GameInstall, textures: &mut UiTextures, images: &mut Assets<Image>) -> Self {
        let mut slots = Vec::new();
        for slot in &FONT_SLOTS {
            let path = format!("FONTS/{}.fnt", slot.name);
            let font = install
                .read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| FontFile::parse(&b).map_err(|e| e.to_string()))
                .and_then(|file| {
                    let image = textures.get(slot.name, images).ok_or("no texture of that name")?;
                    // The US disc's Japanese font pages are 32x32 stubs.
                    let fits = file.glyphs.iter().all(|g| {
                        (g.x + g.width) as f32 <= image.size.x && (g.y + file.height) as f32 <= image.size.y
                    });
                    if !fits {
                        return Err("glyphs fall outside its texture".to_string());
                    }
                    Ok(LoadedFont { file, image, space_width: slot.space_width })
                });
            match font {
                Ok(f) => slots.push(Some(f)),
                Err(why) => {
                    info!("font {}: {why}; Bevy's font stands in", slot.name);
                    slots.push(None);
                }
            }
        }
        let alternates = ALTERNATES.iter().filter_map(|&n| Some((n, textures.get(n, images)?))).collect();
        Self { slots, alternates }
    }

    fn texture(&self, font: &LoadedFont, which: FontTexture) -> UiImage {
        let name = match which {
            FontTexture::Own => None,
            FontTexture::Parch => Some(ALTERNATES[0]),
            FontTexture::Glow => Some(ALTERNATES[1]),
            FontTexture::Gar(n) => Some(ALTERNATES[2 + (n as usize).min(5)]),
        };
        name.and_then(|n| self.alternates.get(n)).cloned().unwrap_or_else(|| font.image.clone())
    }

    /// Height of one line of `slot` at `scale` (the game's line step).
    pub fn line_height(&self, slot: usize, scale: f32) -> f32 {
        let h = self.slots.get(slot).and_then(Option::as_ref).map_or(FALLBACK_HEIGHT[slot.min(12)], |f| f.file.height);
        h as f32 * scale
    }

    /// Width of one line at `scale`.
    pub fn width(&self, slot: usize, scale: f32, text: &str) -> f32 {
        match self.slots.get(slot).and_then(Option::as_ref) {
            Some(f) => f.file.width(&latin1(text), f.space_width) as f32 * scale,
            None => text.chars().count() as f32 * self.line_height(slot, scale) * 0.6,
        }
    }
}

/// Glyph heights of the disc's fonts, for sizing Bevy's font when a game
/// font is missing.
const FALLBACK_HEIGHT: [u32; 13] = [9, 8, 7, 10, 16, 8, 32, 25, 8, 11, 16, 16, 24];

/// The game's strings are single-byte (Latin-1 plus its own symbols: 3 is
/// ©, 0x12 ™).
fn latin1(text: &str) -> Vec<u8> {
    text.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect()
}

/// How a string is drawn.
#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub slot: usize,
    pub scale: f32,
    pub color: Color,
    pub texture: FontTexture,
    /// The game's glow flag: every glyph quad grows by a margin.
    pub glow: bool,
}

impl TextStyle {
    pub fn new(slot: usize, scale: f32, color: Color) -> Self {
        Self { slot, scale, color, texture: FontTexture::Own, glow: false }
    }
    pub fn with_texture(mut self, texture: FontTexture) -> Self {
        self.texture = texture;
        self
    }
    pub fn glowing(mut self) -> Self {
        self.glow = true;
        self
    }
}

/// One thing drawn in the 512 × 384 screen this frame, back to front.
#[derive(Clone, Debug, PartialEq)]
pub enum Quad {
    Image { image: Handle<Image>, rect: Option<Rect>, pos: Vec2, size: Vec2, color: Color },
    /// Stand-in text in Bevy's font for a game font that couldn't load.
    Text { text: String, pos: Vec2, size: f32, color: Color },
}

/// This frame's 2D draw list, cleared every frame.
#[derive(Resource, Default)]
pub struct Draw2d {
    pub quads: Vec<Quad>,
}

impl Draw2d {
    /// An image stretched over `w × h` at `(x, y)`.
    pub fn image(&mut self, image: &UiImage, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.quads.push(Quad::Image {
            image: image.handle.clone(),
            rect: None,
            pos: Vec2::new(x, y),
            size: Vec2::new(w, h),
            color,
        });
    }

    /// A solid rectangle.
    pub fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.quads.push(Quad::Image { image: Handle::default(), rect: None, pos: Vec2::new(x, y), size: Vec2::new(w, h), color });
    }

    /// One line of text; `x < 0` centres it on `-x`, as the game's text
    /// calls do. Returns its width.
    pub fn text(&mut self, fonts: &GameFonts, style: &TextStyle, x: f32, y: f32, text: &str) -> f32 {
        let width = fonts.width(style.slot, style.scale, text);
        let left = if x < 0.0 { -x - (width / 2.0).trunc() } else { x };
        let Some(font) = fonts.slots.get(style.slot).and_then(Option::as_ref) else {
            let size = fonts.line_height(style.slot, style.scale);
            self.quads.push(Quad::Text { text: text.to_string(), pos: Vec2::new(left, y), size, color: style.color });
            return width;
        };
        let image = fonts.texture(font, style.texture);
        // Glyph rectangles are in the font's own texture; another texture
        // with the same layout may be a different size.
        let uv = image.size / font.image.size;
        let height = font.file.height as f32;
        let margin = if style.glow { GLOW_MARGIN } else { 0.0 };
        for placed in font.file.layout(&latin1(text), font.space_width) {
            let Placed::Glyph { glyph, x: pen } = placed else { continue };
            let min = Vec2::new(glyph.x as f32, glyph.y as f32) * uv;
            let max = Vec2::new((glyph.x + glyph.width) as f32, glyph.y as f32 + height) * uv;
            self.quads.push(Quad::Image {
                image: image.handle.clone(),
                rect: Some(Rect::from_corners(min, max)),
                pos: Vec2::new(left + pen as f32 * style.scale - margin, y - margin),
                size: Vec2::new(glyph.width as f32, height) * style.scale + 2.0 * margin,
                color: style.color,
            });
        }
        width
    }

    /// The game's "shimmer" text (Press Start, Loading...): a glow pass in
    /// `glow` whose alpha pulses over `period` fields, then the text in
    /// white on top.
    #[allow(clippy::too_many_arguments)]
    pub fn shimmer(&mut self, fonts: &GameFonts, slot: usize, scale: f32, x: f32, y: f32, text: &str, glow: Color, pulse: f32) {
        let glow = glow.with_alpha(pulse);
        self.text(fonts, &TextStyle::new(slot, scale, glow).with_texture(FontTexture::Glow).glowing(), x, y, text);
        self.text(fonts, &TextStyle::new(slot, scale, Color::WHITE), x, y, text);
    }
}

/// The root the pool's nodes live under, over everything else on screen.
#[derive(Component)]
struct Screen2dRoot;

/// A pooled node showing one [`Quad`].
#[derive(Component)]
struct PooledQuad;

#[derive(Component)]
struct PooledText;

fn load_ui(mut commands: Commands, mut game: ResMut<LoadedGame>, mut images: ResMut<Assets<Image>>) {
    // STATIC: fonts, menu art; TITLE: the title screen; SELECT: character
    // select; CREDITS: the credits font.
    let mut textures = UiTextures::load(&mut game.install, &["STATIC", "TITLE", "SELECT", "CREDITS"]);
    let fonts = GameFonts::load(&mut game.install, &mut textures, &mut images);
    let loaded = fonts.slots.iter().filter(|s| s.is_some()).count();
    info!("{loaded} of {} game fonts loaded", fonts.slots.len());
    commands.insert_resource(textures);
    commands.insert_resource(fonts);
}

fn spawn_root(mut commands: Commands) {
    commands.spawn((
        Screen2dRoot,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        GlobalZIndex(100),
        Pickable::IGNORE,
    ));
}

/// Shows this frame's draw list with the pooled nodes, in order, scaled
/// from the 512 × 384 screen to the largest 4:3 area that fits the window.
#[allow(clippy::type_complexity)]
fn flush(
    mut commands: Commands,
    draw: Res<Draw2d>,
    windows: Query<&Window>,
    root: Query<Entity, With<Screen2dRoot>>,
    mut images: Query<(Entity, &mut Node, &mut ImageNode, &mut Visibility), (With<PooledQuad>, Without<PooledText>)>,
    mut texts: Query<
        (Entity, &mut Node, &mut Text, &mut TextFont, &mut TextColor, &mut Visibility),
        (With<PooledText>, Without<PooledQuad>),
    >,
) {
    let (Ok(window), Ok(root)) = (windows.single(), root.single()) else { return };
    let (w, h) = (window.width(), window.height());
    let scale = (w / SCREEN_W).min(h / SCREEN_H);
    let offset = Vec2::new((w - SCREEN_W * scale) / 2.0, (h - SCREEN_H * scale) / 2.0);
    let place = |node: &mut Node, pos: Vec2, size: Option<Vec2>| {
        let at = offset + pos * scale;
        node.left = Val::Px(at.x);
        node.top = Val::Px(at.y);
        if let Some(size) = size {
            node.width = Val::Px(size.x * scale);
            node.height = Val::Px(size.y * scale);
        }
    };

    let mut image_pool = images.iter_mut().sort_by_key::<Entity, _>(|e| *e);
    let mut text_pool = texts.iter_mut().sort_by_key::<Entity, _>(|e| *e);
    // New nodes are appended to the root, so a pooled node's place in the
    // child list — its stacking — follows the draw order only while the
    // pool is used in order; re-sort children when the pool grows.
    let mut grew = false;
    for quad in &draw.quads {
        match quad {
            Quad::Image { image, rect, pos, size, color } => match image_pool.next() {
                Some((_, mut node, mut img, mut vis)) => {
                    place(&mut node, *pos, Some(*size));
                    if img.image != *image || img.rect != *rect || img.color != *color {
                        img.image = image.clone();
                        img.rect = *rect;
                        img.color = *color;
                    }
                    *vis = Visibility::Inherited;
                }
                None => {
                    let mut node = Node { position_type: PositionType::Absolute, ..default() };
                    place(&mut node, *pos, Some(*size));
                    let img = ImageNode {
                        image: image.clone(),
                        rect: *rect,
                        color: *color,
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    };
                    commands.spawn((PooledQuad, node, img, ChildOf(root), Pickable::IGNORE));
                    grew = true;
                }
            },
            Quad::Text { text, pos, size, color } => match text_pool.next() {
                Some((_, mut node, mut t, mut font, mut c, mut vis)) => {
                    place(&mut node, *pos, None);
                    if t.0 != *text {
                        t.0 = text.clone();
                    }
                    font.font_size = size * scale;
                    c.0 = *color;
                    *vis = Visibility::Inherited;
                }
                None => {
                    let mut node = Node { position_type: PositionType::Absolute, ..default() };
                    place(&mut node, *pos, None);
                    commands.spawn((
                        PooledText,
                        node,
                        Text::new(text.clone()),
                        TextFont { font_size: size * scale, ..default() },
                        TextColor(*color),
                        ChildOf(root),
                        Pickable::IGNORE,
                    ));
                    grew = true;
                }
            },
        }
    }
    for (_, _, _, mut vis) in image_pool {
        *vis = Visibility::Hidden;
    }
    for (_, _, _, _, _, mut vis) in text_pool {
        *vis = Visibility::Hidden;
    }
    if grew {
        // Children draw in list order. Pool nodes are always handed out in
        // entity order, so keep the child list in entity order too.
        commands.queue(move |world: &mut World| {
            let Some(children) = world.get::<Children>(root) else { return };
            let mut sorted: Vec<Entity> = children.iter().collect();
            sorted.sort();
            world.entity_mut(root).detach_all_children().add_children(&sorted);
        });
    }
}
