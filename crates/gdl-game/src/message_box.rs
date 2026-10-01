//! The game's message box and captions (`docs/frontend.md`, "Message box"
//! and "Captions").
//!
//! The box shows scrolls, the tower's gate notices and
//! unlocks, and the tower's welcome: a page of a `TEXT/SCROLL_E.ROM` group
//! at a time on the `Scroll_A` panel, in the group's font and scale in dark
//! brown, with "Press [B] Button when done." shimmering under it. Play
//! stops while it's up (the game runs the box's own loop, drawing the
//! frozen scene); 15 fields into a page, B puts the page away — cutting its
//! voice line short — and the last one closes the box, after which the
//! pads are ignored for 4 frames. `GDL_SKIP_BOXES=1` (testing: scripted
//! runs can't press B) puts each page away as soon as B could.
//!
//! Captions are the wizards' words during their scenes:
//! white, centred, typed out a letter at a time in a cut's black bar, a
//! page after another, each held a second once typed.

use std::collections::VecDeque;

use bevy::prelude::*;
use gdl_formats::font::FONT32;
use gdl_formats::text::{TextGroup, TextRom};

use crate::audio::{QueueVoice, StopSound};
use crate::font::{Draw2d, Flush2d, GameFonts, TextStyle, UiTextures};
use crate::frontend::{self, Frontend};
use crate::level::LoadedGame;

pub struct MessageBoxPlugin;

impl Plugin for MessageBoxPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ShowMessage>()
            .add_message::<ShowCaption>()
            .init_resource::<MessageBox>()
            .init_resource::<Captions>()
            .add_systems(Startup, load_text)
            .add_systems(
                Update,
                (
                    run_box.run_if(crate::online::lockstep_off).after(frontend::read_input).before(frontend::run),
                    run_captions,
                ),
            )
            // Online the box runs on the network's ticks: every machine
            // opens, turns and closes it alike, and any player's B puts a
            // page away (`online.rs`).
            .add_systems(crate::online::NetTick, net_box.before(crate::audio::NetVoices))
            .add_systems(PostUpdate, (draw_box, draw_captions).in_set(DrawBox).before(Flush2d));
    }
}

/// Where the box and the captions are drawn: what goes under them draws
/// before it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct DrawBox;

/// Asks for a message in the box: page `index` of a `TEXT/SCROLL_E.ROM`
/// group (every page in turn when none), with the voice line that plays
/// while it's up — an announcer's line (the tower's unlocks), queued as
/// the box opens.
#[derive(Message, Clone, Debug)]
pub struct ShowMessage {
    pub group: String,
    pub index: Option<usize>,
    pub voice: Option<&'static str>,
}

impl ShowMessage {
    pub fn new(group: impl Into<String>, index: usize) -> Self {
        Self { group: group.into(), index: Some(index), voice: None }
    }

    /// Every page of the group, one after another.
    pub fn all(group: impl Into<String>) -> Self {
        Self { group: group.into(), index: None, voice: None }
    }

    pub fn voice(mut self, line: &'static str) -> Self {
        self.voice = Some(line);
        self
    }
}

/// The text files the box and captions read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFile {
    /// `TEXT/SCROLL_E.ROM`: the tower's texts (the game's bank 0).
    Scroll,
    /// `TEXT/ENGLISH.ROM`: the bosses' speeches (the default bank).
    English,
}

/// Asks for a caption: page `index` of a group (every page in turn when
/// none), typed out centred at line `y` of the 384-line screen — 16 in the
/// top bar for a boss's speech, 312 in the bottom one for the tower's.
/// Each page is held a second once typed and then goes; with `stay`, the
/// last stays until [`Captions::clear`].
#[derive(Message, Clone, Debug)]
pub struct ShowCaption {
    pub file: TextFile,
    pub group: String,
    pub index: Option<usize>,
    pub y: f32,
    pub stay: bool,
    /// A caption made up by the caller (a hero's new rank) instead of the
    /// group's: one page, at the captions' scale.
    pub text: Option<String>,
}

/// Fields a page stays up before B can put it away.
const PAGE_GUARD: f32 = 15.0;
/// Frames the pads are ignored once the box closes.
const QUIET_FRAMES: u32 = 4;
/// A box's voice line is dropped when it would wait longer than this,
/// seconds (the tower's unlock lines).
const VOICE_MOST_WAIT: f32 = 10.0;
/// The panel's texture and the prompt under the text.
const PANEL: &str = "Scroll_A";
const PROMPT: &str = "Press     Button when done.";
const PROMPT_SCALE: f32 = 0.5;
/// The B button's picture in the prompt's gap: x, and its size.
const PROMPT_ICON: &str = "BUTTON_TRI";
const PROMPT_ICON_X: f32 = 190.0;
const PROMPT_ICON_SIZE: f32 = 20.0;
/// The page's letters: dark brown on the parchment.
const INK: [u8; 3] = [0x16, 0x0C, 0x03];
/// Space between lines, and the panel's margins round the text.
const LINE_GAP: f32 = 4.0;
const MARGIN_ACROSS: f32 = 96.0;
const MARGIN_DOWN: f32 = 96.0;
const PROMPT_MARGIN: f32 = 32.0;
/// Where the panel's middle is, and how far below its top the text starts.
const PANEL_MIDDLE: f32 = 160.0;
const TEXT_DROP: f32 = 32.0;
/// The prompt sits this far under the last line.
const PROMPT_DROP: f32 = 8.0;

#[derive(Resource, Default)]
pub struct MessageBox {
    queue: VecDeque<ShowMessage>,
    open: Option<Open>,
    /// Frames the heroes' pads stay ignored after a box closed.
    quiet: u32,
    scroll: Option<TextRom>,
    /// Real fields since the game began, for the prompt's shimmer.
    t: f32,
}

/// The box on screen: its group's pages, the page up, fields it's been up
/// and its voice line.
struct Open {
    group: TextGroup,
    pages: Vec<usize>,
    page: usize,
    fields: f32,
    voice: Option<&'static str>,
}

impl MessageBox {
    /// Nothing up or waiting (a new game online).
    pub fn clear(&mut self) {
        self.queue.clear();
        self.open = None;
        self.quiet = 0;
    }

    /// Whether the box is up (play is frozen).
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Whether the heroes' pads are ignored: the box is up, or just closed.
    pub fn holds_input(&self) -> bool {
        self.open.is_some() || self.quiet > 0
    }
}

#[derive(Resource, Default)]
pub struct Captions {
    queue: VecDeque<ShowCaption>,
    up: Option<Caption>,
    english: Option<TextRom>,
    scroll: Option<TextRom>,
}

impl Captions {
    /// `TEXT/ENGLISH.ROM`, for captions made up from its strings.
    pub fn english(&self) -> Option<&TextRom> {
        self.english.as_ref()
    }

    /// Whether a caption is up with its last page typed.
    pub fn done(&self) -> bool {
        self.up.as_ref().is_some_and(|c| c.done)
    }

    /// Takes the caption down (and any queued).
    pub fn clear(&mut self) {
        self.queue.clear();
        self.up = None;
    }
}

/// A caption being typed: its pages, the one up, ticks since it began and
/// fields it has been held once typed.
struct Caption {
    y: f32,
    scale: f32,
    pages: Vec<String>,
    page: usize,
    ticks: f32,
    held: f32,
    stay: bool,
    /// Its last page is typed (a staying caption then stays).
    done: bool,
}

fn load_text(mut boxes: ResMut<MessageBox>, mut captions: ResMut<Captions>, mut game: ResMut<LoadedGame>) {
    let mut read = |path: &str| match game.install.read(path).map_err(|e| e.to_string()).and_then(|b| TextRom::parse(&b).map_err(|e| e.to_string())) {
        Ok(rom) => Some(rom),
        Err(e) => {
            warn!("{path}: {e}; its messages won't show");
            None
        }
    };
    boxes.scroll = read("TEXT/SCROLL_E.ROM");
    captions.scroll = boxes.scroll.clone();
    captions.english = read("TEXT/ENGLISH.ROM");
}

/// A group by name, as the game finds it (case aside).
fn group<'a>(rom: Option<&'a TextRom>, name: &str) -> Option<&'a TextGroup> {
    rom?.groups.iter().find(|g| g.name.eq_ignore_ascii_case(name))
}

/// Opens queued boxes one after another and turns their pages.
fn run_box(
    real: Res<Time<Real>>,
    mut boxes: ResMut<MessageBox>,
    fe: Res<Frontend>,
    mut requests: MessageReader<ShowMessage>,
    mut voices: MessageWriter<QueueVoice>,
    mut stops: MessageWriter<StopSound>,
) {
    let fields = real.delta_secs() * 60.0;
    step_box(&mut boxes, fields, fe.back_pressed(), &mut requests, &mut voices, &mut stops);
}

/// The box online, on a network tick: two fields, and B from any player.
fn net_box(
    lock: Res<crate::online::Lockstep>,
    inputs: Res<crate::party::Inputs>,
    mut boxes: ResMut<MessageBox>,
    mut requests: MessageReader<ShowMessage>,
    mut voices: MessageWriter<QueueVoice>,
    mut stops: MessageWriter<StopSound>,
) {
    let back = lock.pressed(&inputs, crate::party::SlotInput::BACK);
    step_box(&mut boxes, 2.0, back, &mut requests, &mut voices, &mut stops);
}

/// The box's step: `fields` gone by, `back` B pressed.
fn step_box(
    boxes: &mut MessageBox,
    fields: f32,
    back: bool,
    requests: &mut MessageReader<ShowMessage>,
    voices: &mut MessageWriter<QueueVoice>,
    stops: &mut MessageWriter<StopSound>,
) {
    let skip = std::env::var("GDL_SKIP_BOXES").is_ok_and(|v| !v.is_empty() && v != "0");
    boxes.t += fields;
    boxes.queue.extend(requests.read().cloned());
    if boxes.open.is_none() {
        boxes.quiet = boxes.quiet.saturating_sub(1);
    }
    if let Some(open) = boxes.open.as_mut() {
        open.fields += fields;
        if open.fields >= PAGE_GUARD && (back || skip) {
            if let Some(line) = open.voice {
                stops.write(StopSound(line.into()));
            }
            open.page += 1;
            open.fields = 0.0;
            if open.page >= open.pages.len() {
                boxes.open = None;
                boxes.quiet = QUIET_FRAMES;
            }
        }
        return;
    }
    while let Some(m) = boxes.queue.pop_front() {
        let Some(g) = group(boxes.scroll.as_ref(), &m.group) else {
            warn!("no message group {}", m.group);
            continue;
        };
        let pages: Vec<usize> = match m.index {
            Some(i) if i < g.strings.len() => vec![i],
            Some(i) => {
                warn!("message {} has no page {i}", m.group);
                continue;
            }
            None => (0..g.strings.len()).collect(),
        };
        if pages.is_empty() {
            continue;
        }
        info!("message box: {} {:?}", m.group, pages);
        if let Some(line) = m.voice {
            voices.write(QueueVoice::announcer(line, VOICE_MOST_WAIT));
        }
        boxes.open = Some(Open { group: g.clone(), pages, page: 0, fields: 0.0, voice: m.voice });
        break;
    }
}

/// A page's lines: its line breaks (`\r\n` counts as one) split it.
fn lines(page: &str) -> Vec<String> {
    page.replace("\r\n", "\n").replace('\r', "\n").split('\n').map(|l| l.chars().filter(|&c| c != '\t').collect()).collect()
}

/// Draws the open box: the panel sized to the page (and the prompt), the
/// page centred on it, the prompt and the B button's picture under it.
fn draw_box(
    boxes: Res<MessageBox>,
    fonts: Option<Res<GameFonts>>,
    mut tex: Option<ResMut<UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    mut draw: ResMut<Draw2d>,
) {
    let (Some(open), Some(fonts), Some(tex)) = (boxes.open.as_ref(), fonts, tex.as_deref_mut()) else { return };
    let Some(page) = open.pages.get(open.page).and_then(|&i| open.group.strings.get(i)) else { return };
    let scale = open.group.scale[0];
    let text = lines(page);
    let widest = text.iter().map(|l| fonts.width(FONT32, scale, l)).fold(0.0, f32::max);
    let line = fonts.line_height(FONT32, scale).trunc() + LINE_GAP;
    let height = text.len() as f32 * line;
    let prompt = fonts.width(FONT32, PROMPT_SCALE, PROMPT);
    let w = if prompt + PROMPT_MARGIN <= widest + MARGIN_ACROSS { (widest + MARGIN_ACROSS).min(512.0) } else { prompt + PROMPT_MARGIN };
    let h = height + MARGIN_DOWN;
    let (x, top) = ((256.0 - w / 2.0).trunc(), (PANEL_MIDDLE - h / 2.0).trunc());
    if let Some(panel) = tex.get(PANEL, &mut images) {
        draw.image(&panel, x, top, w, h, Color::WHITE);
    }
    let ink = TextStyle::new(FONT32, scale, Color::srgb_u8(INK[0], INK[1], INK[2]));
    let mut y = top + TEXT_DROP;
    for l in &text {
        draw.text(&fonts, &ink, -256.0, y, l);
        y += line;
    }
    let y = y + PROMPT_DROP;
    draw.shimmer(&fonts, FONT32, PROMPT_SCALE, -256.0, y, PROMPT, frontend::glow_colour(), frontend::pulse(boxes.t));
    if let Some(icon) = tex.get(PROMPT_ICON, &mut images) {
        draw.image(&icon, PROMPT_ICON_X, y, PROMPT_ICON_SIZE, PROMPT_ICON_SIZE, Color::WHITE);
    }
}

/// The captions' typing, in ticks: each letter costs 1.75,
/// a comma or full stop 2 more, a tab 5; a carriage return waits 30 and
/// then clears the lines before it for what follows.
const LETTER: f32 = 1.75;
const STOP: f32 = 2.0;
const TAB: f32 = 5.0;
const RETURN: f32 = 30.0;
/// Fields a typed page stays before the next (`r13-0x77c0`).
const CAPTION_HOLD: f32 = 60.0;
/// Captions' letters at 0.667 of their group's scale, lines 32 apart at
/// that scale.
const CAPTION_SCALE: f32 = 0.667;
const CAPTION_LINE: f32 = 32.0;

/// A caption line on screen: what's typed of it, and the whole line (it
/// stands where the whole line is centred as it types).
#[derive(Debug, PartialEq)]
pub struct TypedLine {
    pub typed: String,
    pub whole: String,
}

/// What of a caption page shows after `budget` ticks — a line slot per
/// line break so far, the last maybe part typed (a carriage return with
/// time left after its wait empties the line it ends, and what follows
/// takes its place) — and whether it's all typed (its end, like a letter,
/// needs time left).
pub fn typed(page: &str, mut budget: f32) -> (Vec<TypedLine>, bool) {
    let mut done: Vec<TypedLine> = Vec::new();
    let mut line = String::new();
    // Where the line being typed starts in `page`, for its whole text.
    let mut start = 0;
    let whole = |from: usize| page[from..].split(['\r', '\n']).next().unwrap_or_default().to_string();
    let mut chars = page.char_indices().peekable();
    loop {
        if budget <= 0.0 {
            if !line.is_empty() {
                done.push(TypedLine { typed: line, whole: whole(start) });
            }
            return (done, false);
        }
        let Some((at, c)) = chars.next() else { break };
        match c {
            '\r' => {
                budget -= RETURN;
                if budget >= 0.0 {
                    line.clear();
                    if chars.peek().is_some_and(|&(_, c)| c == '\n') {
                        chars.next();
                    }
                    start = chars.peek().map_or(page.len(), |&(i, _)| i);
                }
            }
            '\n' => {
                let typed = std::mem::take(&mut line);
                done.push(TypedLine { whole: typed.clone(), typed });
                start = at + 1;
            }
            '\t' => budget -= TAB,
            _ => {
                if c == ',' || c == '.' {
                    budget -= STOP;
                }
                budget -= LETTER;
                line.push(c);
            }
        }
    }
    if !line.is_empty() {
        done.push(TypedLine { whole: line.clone(), typed: line });
    }
    (done, true)
}

/// Types queued captions one after another.
fn run_captions(
    time: Res<Time>,
    mut captions: ResMut<Captions>,
    mut requests: MessageReader<ShowCaption>,
) {
    captions.queue.extend(requests.read().cloned());
    let fields = time.delta_secs() * 60.0;
    if let Some(c) = captions.up.as_mut() {
        c.ticks += fields / 2.0;
        let page = &c.pages[c.page];
        if typed(page, c.ticks).1 {
            let last = c.page + 1 == c.pages.len();
            c.done |= last;
            if last && c.stay {
                return;
            }
            c.held += fields;
            if c.held >= CAPTION_HOLD {
                c.page += 1;
                c.ticks = 0.0;
                c.held = 0.0;
                if c.page >= c.pages.len() {
                    captions.up = None;
                }
            }
        }
        return;
    }
    while let Some(r) = captions.queue.pop_front() {
        if let Some(text) = r.text {
            info!("caption: {}", text.replace(['\n', '\r', '\t'], " "));
            let pages = vec![text];
            captions.up = Some(Caption { y: r.y, scale: CAPTION_SCALE, pages, page: 0, ticks: 0.0, held: 0.0, stay: r.stay, done: false });
            break;
        }
        let rom = match r.file {
            TextFile::Scroll => captions.scroll.as_ref(),
            TextFile::English => captions.english.as_ref(),
        };
        let Some(g) = group(rom, &r.group) else {
            warn!("no caption group {}", r.group);
            continue;
        };
        let pages: Vec<String> = match r.index {
            Some(i) => g.strings.get(i).cloned().into_iter().collect(),
            None => g.strings.clone(),
        };
        if pages.is_empty() {
            continue;
        }
        info!("caption: {} {}", r.group, pages.join(" ").replace(['\n', '\r', '\t'], " "));
        let scale = g.scale[0] * CAPTION_SCALE;
        captions.up = Some(Caption { y: r.y, scale, pages, page: 0, ticks: 0.0, held: 0.0, stay: r.stay, done: false });
        break;
    }
}

fn draw_captions(captions: Res<Captions>, fonts: Option<Res<GameFonts>>, mut draw: ResMut<Draw2d>, fe: Option<Res<Frontend>>) {
    // Not over a screen that covers play (the shop, the select screen).
    if fe.is_some_and(|f| f.covers_play()) {
        return;
    }
    let (Some(c), Some(fonts)) = (captions.up.as_ref(), fonts) else { return };
    let Some(page) = c.pages.get(c.page) else { return };
    let style = TextStyle::new(FONT32, c.scale, Color::WHITE);
    let mut y = c.y;
    for line in typed(page, c.ticks).0 {
        if !line.typed.is_empty() {
            let left = 256.0 - (fonts.width(FONT32, c.scale, &line.whole) / 2.0).trunc();
            draw.text(&fonts, &style, left, y, &line.typed);
        }
        y += CAPTION_LINE * c.scale;
    }
}

/// How long a caption takes to type and hold, in seconds: pages in turn.
pub fn caption_seconds(pages: &[String]) -> f32 {
    pages
        .iter()
        .map(|p| {
            let mut ticks = 0.0;
            while !typed(p, ticks).1 {
                ticks += 1.0;
            }
            ticks / 30.0 + CAPTION_HOLD / 60.0
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(lines: &[TypedLine]) -> Vec<&str> {
        lines.iter().map(|l| l.typed.as_str()).collect()
    }

    #[test]
    fn captions_type_by_the_letter_and_clear_at_a_return() {
        let page = "Congratulations!\r\nYou have recovered the Shard\nfor the Forsaken Province.";
        // A letter types while there's time left before it: the 16th from
        // 15 × 1.75 ticks on.
        assert_eq!(shown(&typed(page, 26.2).0), vec!["Congratulations"]);
        assert_eq!(shown(&typed(page, 26.3).0), vec!["Congratulations!"]);
        let (lines, done) = typed(page, 28.0 + 0.1);
        assert_eq!(shown(&lines), vec!["Congratulations!"]);
        assert_eq!(lines[0].whole, "Congratulations!");
        assert!(!done);
        // Past the return's wait, its line is gone and the next types in
        // its place, standing where the whole line will be.
        let (lines, _) = typed(page, 28.0 + 30.0 + 1.75 * 3.0 + 0.1);
        assert_eq!(shown(&lines), vec!["You "]);
        assert_eq!(lines[0].whole, "You have recovered the Shard");
        let (lines, done) = typed(page, 1000.0);
        assert_eq!(shown(&lines), vec!["You have recovered the Shard", "for the Forsaken Province."]);
        assert!(done);
        // 70 letters, a return and one full stop: its end needs time left
        // too.
        let cost = 70.0 * LETTER + RETURN + STOP;
        assert!(!typed(page, cost).1);
        assert!(typed(page, cost + 0.01).1);
    }

    #[test]
    fn pages_split_on_their_breaks() {
        assert_eq!(lines("You need 15 Orange Crystals\nto enter."), vec!["You need 15 Orange Crystals", "to enter."]);
        assert_eq!(lines("A\r\nB"), vec!["A", "B"]);
    }
}
