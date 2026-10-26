//! The pause state in the client: the game's Iggy pause movie, opened by the
//! keys bound to PAUSE and closed by the movie's `ResumeGame`.
//!
//! Host integration: the movie runs on its own thread with a large stack (the
//! interpreter nests AS3 calls on the native stack); the main thread sends it
//! input and frame ticks and receives draw lists.
use crate::assets::{archive::Catalog, installation, lang};
use crate::flash::{player::CharRef, raster, render, Player};
use crate::gameplay::game_clock_runtime::GamePaused;
use crate::gameplay::pause_state::{self, Character, Movie, Request};
use crate::platform::bindings::{self, Keyboard};
use crate::platform::game_settings;
use crate::rendering::bevy_backend::flash_ui::{FlashFrame, FlashUiFrame, Image};
use bevy::input::{
    keyboard::KeyboardInput,
    mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel},
    ButtonState,
};
use bevy::window::{CursorGrabMode, CursorOptions};
use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// `GBA_PAUSE`.
const PAUSE_ACTION: i32 = 43;

enum Command {
    /// While closed: build the next open's player ahead of time.
    Prepare { screen: (i32, i32) },
    /// OnEnter: load the movie (the decl does not cache it) and show it.
    Open { screen: (i32, i32) },
    /// An engine key code, down or up.
    Key { down: bool, key: u32 },
    /// The cursor in movie pixels (`FUN_140033530`).
    Cursor { x: f64, y: f64 },
    /// Engine mouse button code 0xe0..0xe6 (`FUN_140033450`).
    MouseButton { code: u32, down: bool },
    /// Wheel notches (Iggy type 8).
    Wheel(i32),
    /// The frame's time step, and when the frame sent it.
    Tick(f64, std::time::Instant),
    /// OnExit: the movie is unloaded (`m_cacheResources` is 0).
    Close,
    /// Diagnostic (F11, like the F8 capture): write the display trees and the
    /// draw list to `artifacts/flash-dumps/`, with the window size.
    DebugDump { window: (u32, u32) },
}

enum Event {
    Frame(FlashFrame),
    Sound(u32),
    Resume,
    Quit,
    ScreenFx(bool),
    Failed(String),
}

struct Loaded {
    player: Player,
    pause: pause_state::Pause,
    /// Last cursor position sent (`rec+0xb08`, initially 1e30).
    cursor: (f64, f64),
    /// The first keyboard key pressed since the last tick, else the first
    /// mouse button (the options key capture reads it).
    pressed: Option<u32>,
    /// The pause movie's bitmaps, and those plus the open box's global
    /// movie (shifted by `LAYER_LIBRARY_OFFSET`).
    pause_images: HashMap<CharRef, Arc<Image>>,
    images: Arc<HashMap<CharRef, Arc<Image>>>,
    /// The focus entries (`iggyMain+0x18`); input goes to the top one.
    focus: Vec<Movie>,
    pending: f64,
    frame_interval: f64,
    /// Draw-list reuse for the pause movie and the message box movie.
    draw_cache: render::Cache,
    global_cache: render::Cache,
    /// The UI screen size (the display the glyph cache pages are sized for).
    screen: (i32, i32),
}

thread_local! {
    // What the movie thread keeps between opens: the installed catalog and
    // each library resource's decoded bitmaps (the files do not change while
    // the game runs). Every open still builds a new player, as the game does
    // (`m_cacheResources` 0).
    static CATALOG: std::cell::RefCell<Option<std::rc::Rc<Catalog>>> = const { std::cell::RefCell::new(None) };
    static DECODED: std::cell::RefCell<HashMap<String, std::rc::Rc<Vec<(u16, Arc<Image>)>>>> = std::cell::RefCell::new(HashMap::new());
}

fn installed_catalog() -> Result<std::rc::Rc<Catalog>, String> {
    if let Some(c) = CATALOG.with(|c| c.borrow().clone()) {
        return Ok(c);
    }
    let base = installation::game_base().map_err(|e| e.to_string())?;
    let c = std::rc::Rc::new(Catalog::open(&base).map_err(|e| e.to_string())?);
    CATALOG.with(|slot| *slot.borrow_mut() = Some(c.clone()));
    Ok(c)
}

fn decode(catalog: &Catalog, player: &Player, resources: &[(usize, String)]) -> Result<HashMap<CharRef, Arc<Image>>, String> {
    let mut out = HashMap::new();
    for (index, path) in resources {
        let images = match DECODED.with(|d| d.borrow().get(path).cloned()) {
            Some(images) => images,
            None => {
                let mut decoded = raster::Images::default();
                let file = player.state.borrow().file(*index);
                decoded.load(catalog, *index, path, &file)?;
                let images = std::rc::Rc::new(decoded.images.into_iter().map(|(c, (width, height, rgba))| (c.id, Arc::new(Image { width, height, rgba }))).collect::<Vec<_>>());
                DECODED.with(|d| d.borrow_mut().insert(path.clone(), images.clone()));
                images
            }
        };
        out.extend(images.iter().map(|(id, image)| (CharRef { library: *index, id: *id }, image.clone())));
    }
    Ok(out)
}

impl Loaded {
    /// The movie of the top focus entry.
    fn target(&mut self) -> &mut Player {
        if self.focus.last() == Some(&Movie::Global) {
            if let Some(g) = self.pause.boxes.global() {
                return g;
            }
        }
        &mut self.player
    }

    /// The pause images plus the front box's global movie.
    fn rebuild_images(&mut self) -> Result<(), String> {
        let mut images = self.pause_images.clone();
        if let Some((g, resources)) = self.pause.boxes.global_with_resources() {
            let catalog = installed_catalog()?;
            for (mut c, image) in decode(&catalog, g, resources)? {
                c.library += render::LAYER_LIBRARY_OFFSET;
                images.insert(c, image);
            }
        }
        self.images = Arc::new(images);
        Ok(())
    }
}

/// Main-thread owner of the pause state.
pub struct PauseMenu {
    commands: Sender<Command>,
    events: Receiver<Event>,
    open: bool,
    /// The screen size the next player was last prepared for.
    prepared_for: Option<(i32, i32)>,
    /// `DH2_DEBUG_OPEN_PAUSE_AFTER` has opened the pause state.
    debug_opened: bool,
    /// The next `DH2_DEBUG_PAUSE_INPUT` action.
    debug_step: usize,
    /// Diagnostic frame-time accumulation (`DH2_DEBUG_PAUSE_TIMING`).
    debug_frames: (u32, f64, Option<std::time::Instant>),
    /// The movie has produced a frame since it was opened: each frame then
    /// waits for its own tick's draw list.
    ready: bool,
}

/// The longest a frame waits for the movie's draw list (a slow tick is
/// shown on a later frame instead).
const FRAME_WAIT: std::time::Duration = std::time::Duration::from_millis(100);

impl Default for PauseMenu {
    fn default() -> Self {
        let (commands, inbox) = channel();
        let (outbox, events) = channel();
        std::thread::Builder::new()
            .name("pause movie".into())
            .stack_size(256 << 20)
            .spawn(move || run(inbox, outbox))
            .expect("pause movie thread");
        Self { commands, events, open: false, ready: false, prepared_for: None, debug_opened: false, debug_step: 0, debug_frames: (0, 0.0, None) }
    }
}

/// The player character. The rewrite has no save state yet, so the character
/// cannot come from the game; `DH2_DEBUG_CHARACTER=emily` selects Emily's
/// movie and player declarations for testing, Corvo otherwise.
pub(super) fn debug_character() -> Character {
    match std::env::var("DH2_DEBUG_CHARACTER").as_deref() {
        Ok("emily") => Character::Emily,
        _ => Character::Corvo,
    }
}

/// A player built before the open that uses it: the movie and its libraries
/// loaded at the screen size, and its bitmaps decoded. Nothing in it has
/// reached the game yet; its clock restarts when it is taken.
struct Prepared {
    screen: (i32, i32),
    player: Player,
    images: HashMap<CharRef, Arc<Image>>,
    options: crate::gameplay::options::Options,
}

fn prepare(screen: (i32, i32)) -> Result<Prepared, String> {
    let base = installation::game_base().map_err(|e| e.to_string())?;
    let catalog = installed_catalog()?;
    let language = lang::read_text_language(&base)?;
    let movie = pause_state::movie(debug_character());
    let (mut player, resources) = Player::load_installed(&catalog, language, movie)?;
    pause_state::file_ready(&mut player, screen)?;
    let images = decode(&catalog, &player, &resources)?;
    let options = pause_state::read_options(&catalog)?;
    // The chapter header's map lookup, cached per map.
    crate::gameplay::map_info::current_loading_info(&catalog);
    Ok(Prepared { screen, player, images, options })
}

fn load(screen: (i32, i32), prepared: Option<Prepared>) -> Result<Loaded, String> {
    let started = std::time::Instant::now();
    let mut phases: Vec<(&str, std::time::Instant)> = vec![("start", started)];
    let base = installation::game_base().map_err(|e| e.to_string())?;
    let catalog = installed_catalog()?;
    let language = lang::read_text_language(&base)?;
    let movie = pause_state::movie(debug_character());
    let Prepared { mut player, images: pause_images, options, .. } = match prepared.filter(|p| p.screen == screen) {
        Some(p) => p,
        None => prepare(screen)?,
    };
    // `getTimer` counts from the open, as for a player created now.
    player.vm.started = std::time::Instant::now();
    phases.push(("movie, libraries and images", std::time::Instant::now()));
    let state = pause_state::current_state(&player, &language, &catalog);
    let config = game_settings::configuration_path().ok();
    // A message box loads its own `global.iggy` (`FUN_1404627e0`).
    let load_global = Box::new(move || {
        let catalog = installed_catalog()?;
        let (mut g, resources) = Player::load_installed(&catalog, language, crate::gameplay::message_box::MOVIE)?;
        pause_state::file_ready(&mut g, screen)?;
        Ok((g, resources))
    });
    let mut pause = pause_state::Pause::with_options(&mut player, options, &base, state, config, load_global)?;
    phases.push(("pause state", std::time::Instant::now()));
    pause.on_flash_loaded(&mut player)?;
    pause.show(&mut player)?;
    phases.push(("loaded and shown", std::time::Instant::now()));
    if std::env::var_os("DH2_DEBUG_PAUSE_TIMING").is_some_and(|v| v == "1") {
        let parts: Vec<String> = phases.windows(2).map(|w| format!("{} {:.0} ms", w[1].0, (w[1].1 - w[0].1).as_secs_f64() * 1000.0)).collect();
        info!("Pause open timing: {}", parts.join(", "));
    }
    let frame_interval = 1.0 / player.state.borrow().frame_rate;
    info!("Pause movie {movie} ({language}) loaded in {:.2} s", started.elapsed().as_secs_f64());
    Ok(Loaded {
        player,
        pause,
        cursor: (1e30, 1e30),
        pressed: None,
        images: Arc::new(pause_images.clone()),
        pause_images,
        focus: Vec::new(),
        pending: 0.0,
        frame_interval,
        draw_cache: {
            let mut cache = render::Cache::default();
            cache.set_font_caching(pause_state::font_caching(&catalog, movie, (screen.0 as u32, screen.1 as u32)));
            cache
        },
        global_cache: render::Cache::default(),
        screen,
    })
}

fn report_uncaught(player: &mut Player) {
    let uncaught = std::mem::take(&mut player.state.borrow_mut().uncaught);
    for e in uncaught {
        warn!("Pause movie: {e}");
    }
}

/// Everything queued since the thread last looked, in order, with each run
/// of cursor moves reduced to its last position and the frames' ticks
/// merged into the last one (their time steps summed, the first frame's
/// send time kept): input is never processed behind a backlog of frames.
fn batch(inbox: &Receiver<Command>) -> Option<Vec<Command>> {
    let first = inbox.recv().ok()?;
    let mut out: Vec<Command> = Vec::new();
    let mut tick: Option<(f64, std::time::Instant)> = None;
    for command in std::iter::once(first).chain(inbox.try_iter()) {
        match command {
            Command::Tick(dt, sent) => {
                tick = Some(tick.map_or((dt, sent), |(sum, first)| (sum + dt, first)));
            }
            Command::Cursor { .. } if matches!(out.last(), Some(Command::Cursor { .. })) => *out.last_mut().unwrap() = command,
            // A reload or unload ends the batch's earlier frames.
            Command::Open { .. } | Command::Close => {
                tick = None;
                out.push(command);
            }
            other => out.push(other),
        }
    }
    out.extend(tick.map(|(dt, sent)| Command::Tick(dt, sent)));
    Some(out)
}

/// The movie thread: one player at a time, rebuilt on every Open.
fn run(inbox: Receiver<Command>, outbox: Sender<Event>) {
    let mut loaded: Option<Loaded> = None;
    let keyboard = Keyboard::default();
    let mut generation = 0u64;
    let mut serial = 0u64;
    let mut unhandled = Vec::new();
    let mut timing = debug_timing::Timing::default();
    let mut prepared: Option<Prepared> = None;
    // The last frame sent; it is sent again while nothing changed the movie
    // (no tick, input, notification or request), and keeps its uploaded vertices.
    let mut last_frame: Option<FlashFrame> = None;
    while let Some(commands) = batch(&inbox) {
      let batch_started = std::time::Instant::now();
      let mut changed = false;
      for command in commands {
        match command {
            Command::Prepare { screen } => {
                if loaded.is_none() && prepared.as_ref().is_none_or(|p| p.screen != screen) {
                    match prepare(screen) {
                        Ok(p) => prepared = Some(p),
                        Err(e) => warn!("Pause movie: {e}"),
                    }
                }
            }
            Command::Open { screen } => {
                generation += 1;
                last_frame = None;
                match load(screen, prepared.take()) {
                    Ok(l) => loaded = Some(l),
                    Err(e) => {
                        let _ = outbox.send(Event::Failed(e));
                    }
                }
            }
            Command::Close => loaded = None,
            Command::DebugDump { window } => {
                let Some(l) = loaded.as_mut() else { continue };
                let mut out = format!("window {window:?}\n== pause movie\n");
                if let Some(stage) = l.player.state.borrow().stage.clone() {
                    crate::flash::display::debug_tree(&stage, 0, &mut out);
                }
                if let Some(g) = l.pause.boxes.global() {
                    out.push_str("== message box movie\n");
                    if let Some(stage) = g.state.borrow().stage.clone() {
                        crate::flash::display::debug_tree(&stage, 0, &mut out);
                    }
                }
                if let Some(frame) = &last_frame {
                    out.push_str(&format!("== draw list: {} batches, {} vertices, {} layers\n", frame.list.batches.len(), frame.list.vertices.len(), frame.list.layers.len()));
                    for (i, b) in frame.list.batches.iter().enumerate() {
                        let verts = &frame.list.vertices[b.first as usize..(b.first + b.count) as usize];
                        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                        for v in verts {
                            x0 = x0.min(v.position[0]);
                            y0 = y0.min(v.position[1]);
                            x1 = x1.max(v.position[0]);
                            y1 = y1.max(v.position[1]);
                        }
                        out.push_str(&format!("batch {i}: {:?} blend {} count {} stencil test {} set {} clear {} bounds [{x0:.1} {y0:.1} {x1:.1} {y1:.1}] mul {:?}\n", b.texture, b.blend, b.count, b.stencil_test, b.stencil_set, b.stencil_clear, verts.first().map(|v| v.mul)));
                    }
                    for l in &frame.list.layers {
                        out.push_str(&format!("layer {}..{} bounds {:?} filters {:?}\n", l.first, l.end, l.bounds, l.filters));
                    }
                }
                let directory = super::state::diagnostic_directory("flash-dumps");
                let path = directory.join(format!("flash-{}x{}-{}.txt", window.0, window.1, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())));
                match std::fs::create_dir_all(&directory).and_then(|_| std::fs::write(&path, out)) {
                    Ok(()) => info!("Flash dump written to {}", path.display()),
                    Err(e) => warn!("Flash dump {}: {e}", path.display()),
                }
            }
            Command::Key { down, key } => {
                let Some(l) = loaded.as_mut() else { continue };
                if down && !l.pressed.is_some_and(|k| k < 0xe0) {
                    l.pressed = Some(key);
                }
                // `FUN_140033740`: engine keys as virtual keys (`FUN_140032f20`), keyLocation 0.
                let vk = keyboard.virtual_key(key).unwrap_or(key);
                changed = true;
                let target = l.target();
                if let Err(e) = target.key_event(down, vk, 0) {
                    warn!("Pause movie key: {}", target.describe(&e));
                }
                report_uncaught(target);
            }
            Command::Cursor { x, y } => {
                let Some(l) = loaded.as_mut() else { continue };
                // Moves of more than 0.1 movie pixel move `mouseCursor` and send
                // the Iggy move event.
                if (l.cursor.0 - x).abs() > 0.1 || (l.cursor.1 - y).abs() > 0.1 {
                    l.cursor = (x, y);
                    changed = true;
                    let target = l.target();
                    let moved = target
                        .set_value("mouseCursor.x", x.into())
                        .and_then(|_| target.set_value("mouseCursor.y", y.into()))
                        .and_then(|_| target.mouse_move(x, y));
                    if let Err(e) = moved {
                        warn!("Pause movie cursor: {}", target.describe(&e));
                    }
                    report_uncaught(target);
                }
            }
            Command::MouseButton { code, down } => {
                let Some(l) = loaded.as_mut() else { continue };
                if down && l.pressed.is_none() {
                    l.pressed = Some(code);
                }
                changed = true;
                // The Iggy button event first (left/right/middle), then
                // `IG_OnMouseEvent(code, state)` for every button.
                let target = l.target();
                let mut result = Ok(());
                if (0xe0..=0xe2).contains(&code) {
                    result = target.mouse_button((code - 0xe0) as u8, down);
                }
                let state = if down { 1.0 } else { 0.0 };
                let result = result.and_then(|_| {
                    target.call_path_method("inputsManager.mouseHandler", "IG_OnMouseEvent", &[(code as f64).into(), state.into()]).map(|_| ())
                });
                if let Err(e) = result {
                    warn!("Pause movie mouse: {}", target.describe(&e));
                }
                report_uncaught(target);
            }
            Command::Wheel(delta) => {
                let Some(l) = loaded.as_mut() else { continue };
                changed = true;
                let target = l.target();
                if let Err(e) = target.mouse_wheel(delta) {
                    warn!("Pause movie wheel: {}", target.describe(&e));
                }
                report_uncaught(target);
            }
            Command::Tick(dt, sent) => {
                let Some(l) = loaded.as_mut() else { continue };
                let tick_started = std::time::Instant::now();
                let mut phases = [std::time::Duration::ZERO; 4];
                phases[0] = tick_started - batch_started;
                // `IggyPlayerReadyToTick`/`FUN_141021b40`: one tick per frame interval.
                l.pending += dt;
                if l.pending >= l.frame_interval {
                    changed = true;
                    let step = l.pending.min(l.frame_interval);
                    l.pending = (l.pending - l.frame_interval).max(0.0);
                    if let Err(e) = l.player.tick(step) {
                        warn!("Pause movie tick: {}", l.player.describe(&e));
                    }
                    report_uncaught(&mut l.player);
                    if let Some(g) = l.pause.boxes.global() {
                        if let Err(e) = g.tick(step) {
                            warn!("Message box movie tick: {}", g.describe(&e));
                        }
                        report_uncaught(g);
                    }
                }
                phases[1] = tick_started.elapsed();
                let pressed = l.pressed.take();
                changed |= pressed.is_some();
                if let Err(e) = l.pause.update(&mut l.player, pressed) {
                    warn!("Pause options: {e}");
                }
                let handled = l.pause.handle_notifications(&mut l.player, &mut unhandled).and_then(|mut r| {
                    r.extend(l.pause.handle_global_notifications(&mut l.player, &mut unhandled)?);
                    Ok(r)
                });
                match handled {
                    Ok(requests) => {
                        changed |= !requests.is_empty();
                        for request in requests {
                            match request {
                                Request::Resume => {
                                    let _ = outbox.send(Event::Resume);
                                }
                                Request::Quit => {
                                    let _ = outbox.send(Event::Quit);
                                }
                                Request::ScreenFx(on) => {
                                    let _ = outbox.send(Event::ScreenFx(on));
                                }
                                Request::SoundEvent(id) => {
                                    let _ = outbox.send(Event::Sound(id));
                                }
                                // The window follows the cvars (`display_settings`).
                                Request::DisplayChanged => {}
                                // `FUN_140033b50` / `FUN_140034360`.
                                Request::Focus(movie, true) => l.focus.push(movie),
                                Request::Focus(movie, false) => l.focus.retain(|m| *m != movie),
                                Request::GlobalChanged => {
                                    generation += 1;
                                    l.global_cache = render::Cache::default();
                                    if let Ok(catalog) = installed_catalog() {
                                        let display = (l.screen.0 as u32, l.screen.1 as u32);
                                        l.global_cache.set_font_caching(pause_state::font_caching(&catalog, crate::gameplay::message_box::MOVIE, display));
                                    }
                                    if let Err(e) = l.rebuild_images() {
                                        warn!("Message box movie: {e}");
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => warn!("Pause movie: {e}"),
                }
                changed |= !unhandled.is_empty();
                for name in unhandled.drain(..) {
                    debug!("Pause movie notification not handled yet: {name}");
                }
                phases[2] = tick_started.elapsed() - phases[1];
                // Diagnostic: DH2_DEBUG_FLASH_ALL=1 also draws hidden objects.
                let frame = match last_frame.as_ref().filter(|f| !changed && f.generation == generation) {
                    Some(f) => f.clone(),
                    None => {
                        let debug_all = std::env::var_os("DH2_DEBUG_FLASH_ALL").is_some_and(|v| v == "1");
                        let mut list = if debug_all { render::debug_draw_all(&l.player.vm) } else { render::draw_list_cached(&l.player.vm, &mut l.draw_cache) };
                        if let Some(g) = l.pause.boxes.global() {
                            // The message box player is replaced on GlobalChanged.
                            render::stack(&mut list, render::draw_list_cached(&g.vm, &mut l.global_cache));
                        }
                        serial += 1;
                        FlashFrame { list: Arc::new(list), images: l.images.clone(), generation, serial }
                    }
                };
                last_frame = Some(frame.clone());
                changed = false;
                let _ = outbox.send(Event::Frame(frame));
                phases[3] = tick_started.elapsed() - phases[1] - phases[2];
                timing.record(batch_started.duration_since(sent), batch_started.elapsed(), phases);
            }
        }
      }
    }
}

/// The PAUSE binding opens the pause state; while it is open the game is
/// paused, keys go to the movie and its draw list is published for the UI
/// pass. `ResumeGame` from the movie closes it.
#[allow(clippy::too_many_arguments)]
pub fn update_pause_menu(
    mut menu: NonSendMut<PauseMenu>,
    mut keys: MessageReader<KeyboardInput>,
    mut buttons: MessageReader<MouseButtonInput>,
    mut wheels: MessageReader<MouseWheel>,
    time: Res<Time>,
    windows: Query<&Window>,
    mut cursors: Query<&mut CursorOptions>,
    mut paused: ResMut<GamePaused>,
    mut frame: ResMut<FlashUiFrame>,
    mut exit: MessageWriter<AppExit>,
    mut screen_fx: Option<ResMut<super::pause_fx::PauseScreenFx>>,
    mut sound_events: MessageWriter<super::audio::SoundEvent>,
) {
    let menu = &mut *menu;
    // Diagnostic: DH2_DEBUG_OPEN_PAUSE_AFTER=<seconds> opens the pause state
    // once, that long after start (captures without keyboard input).
    let debug_open = !menu.debug_opened
        && std::env::var("DH2_DEBUG_OPEN_PAUSE_AFTER").ok().and_then(|v| v.parse::<f64>().ok()).is_some_and(|after| time.elapsed_secs_f64() >= after);
    if debug_open && !menu.open {
        menu.debug_opened = true;
        let size = windows.iter().next().map_or((1920, 1080), |w| (w.physical_width(), w.physical_height()));
        let _ = menu.commands.send(Command::Open { screen: pause_state::screen_size(size, size) });
        menu.open = true;
        menu.ready = false;
        menu.prepared_for = None;
    }
    // Diagnostic: DH2_DEBUG_PAUSE_INPUT="click:X,Y@SECONDS;move:X,Y@SECONDS"
    // moves to movie pixel (X, Y) and clicks, or only moves, that long after
    // start (with the menu open), in order.
    if menu.open {
        let actions: Vec<(bool, f64, f64, f64)> = std::env::var("DH2_DEBUG_PAUSE_INPUT")
            .unwrap_or_default()
            .split(';')
            .filter_map(|a| {
                let (kind, rest) = a.trim().split_once(':')?;
                let (xy, at) = rest.split_once('@')?;
                let (x, y) = xy.split_once(',')?;
                Some((kind == "click", x.parse().ok()?, y.parse().ok()?, at.parse().ok()?))
            })
            .collect();
        while let Some(&(click, x, y, _)) = actions.get(menu.debug_step).filter(|a| time.elapsed_secs_f64() >= a.3) {
            menu.debug_step += 1;
            let _ = menu.commands.send(Command::Cursor { x, y });
            if click {
                let _ = menu.commands.send(Command::MouseButton { code: 0xe0, down: true });
                let _ = menu.commands.send(Command::MouseButton { code: 0xe0, down: false });
            }
        }
    }
    if std::env::var_os("DH2_DEBUG_PAUSE_TIMING").is_some_and(|v| v == "1") {
        let (frames, total, since) = &mut menu.debug_frames;
        *frames += 1;
        *total += time.delta_secs_f64();
        let since = *since.get_or_insert_with(std::time::Instant::now);
        if since.elapsed().as_secs_f64() >= 1.0 {
            info!("Frame timing (pause open: {}): {} frames, avg {:.1} ms", menu.open, *frames, *total * 1000.0 / *frames as f64);
            menu.debug_frames = (0, 0.0, None);
        }
    }
    for input in keys.read() {
        // Engine keyboard events do not repeat.
        if input.repeat {
            continue;
        }
        // Diagnostic: F11 dumps the open menu; the key still reaches the movie.
        if menu.open && input.key_code == KeyCode::F11 && input.state == ButtonState::Pressed {
            let window = windows.iter().next().map_or((0, 0), |w| (w.physical_width(), w.physical_height()));
            let _ = menu.commands.send(Command::DebugDump { window });
        }
        let Some(key) = super::engine_keys::engine_key(input.key_code) else { continue };
        let down = input.state == ButtonState::Pressed;
        if menu.open {
            let _ = menu.commands.send(Command::Key { down, key });
        } else if down && crate::platform::console::shared().read().bindings.keys_for_action(bindings::IN_GAME, PAUSE_ACTION).contains(&key) {
            // `SwitchState(GAME_LOGIC_STATE_PAUSE)`; the UI renders at window size.
            let size = windows.iter().next().map_or((1920, 1080), |w| (w.physical_width(), w.physical_height()));
            let screen = pause_state::screen_size(size, size);
            let _ = menu.commands.send(Command::Open { screen });
            menu.open = true;
            menu.ready = false;
            // The prepared player is used up; prepare again after closing.
            menu.prepared_for = None;
        }
    }
    if menu.open {
        if let Some(window) = windows.iter().next() {
            // The UI fills the window: movie pixels = window pixels × 1920 / width.
            if let Some(c) = window.cursor_position() {
                let x = c.x as f64 * 1920.0 / window.width() as f64;
                let y = c.y as f64 * 1080.0 / window.height() as f64;
                let _ = menu.commands.send(Command::Cursor { x, y });
            }
        }
        for b in buttons.read() {
            // DirectInput buttons 0..6 are engine codes 0xe0..0xe6.
            let code = match b.button {
                MouseButton::Left => 0xe0,
                MouseButton::Right => 0xe1,
                MouseButton::Middle => 0xe2,
                MouseButton::Back => 0xe3,
                MouseButton::Forward => 0xe4,
                MouseButton::Other(n) if n < 7 => 0xe0 + n as u32,
                _ => continue,
            };
            let _ = menu.commands.send(Command::MouseButton { code, down: b.state == ButtonState::Pressed });
        }
        for w in wheels.read() {
            let notches = match w.unit {
                MouseScrollUnit::Line => w.y as i32,
                MouseScrollUnit::Pixel => (w.y / 120.0) as i32,
            };
            if notches != 0 {
                let _ = menu.commands.send(Command::Wheel(notches.signum()));
            }
        }
        let _ = menu.commands.send(Command::Tick(time.delta_secs_f64(), std::time::Instant::now()));
    } else {
        buttons.clear();
        wheels.clear();
    }
    // The game ticks its Flash players within the frame, so this frame shows
    // this frame's input: once the movie is loaded, wait for the draw list
    // of the tick just sent.
    let waiting = menu.open && menu.ready;
    let mut framed = false;
    loop {
        let event = match menu.events.try_recv() {
            Ok(e) => e,
            Err(_) if waiting && !framed => match menu.events.recv_timeout(FRAME_WAIT) {
                Ok(e) => e,
                Err(_) => break,
            },
            Err(_) => break,
        };
        match event {
            Event::Frame(f) if menu.open => {
                framed = true;
                menu.ready = true;
                frame.0 = Some(f);
            }
            Event::Frame(_) => {}
            Event::Resume => {
                // `FUN_14078ca40` pops the state; OnExit unloads the movie.
                let _ = menu.commands.send(Command::Close);
                menu.open = false;
            }
            // The `quit` command ends the application.
            Event::Quit => {
                exit.write(AppExit::Success);
            }
            Event::ScreenFx(on) => {
                if let Some(fx) = screen_fx.as_mut() {
                    fx.requested = Some(on);
                }
            }
            Event::Sound(id) => {
                sound_events.write(super::audio::SoundEvent(crate::gameplay::sound::Request::Post {
                    event: id, emitter: crate::gameplay::sound::Emitter::Menu,
                }));
            }
            Event::Failed(e) => {
                error!("Pause menu unavailable: {e}");
                menu.open = false;
            }
        }
    }
    if !menu.open {
        frame.0 = None;
        // Build the next open's player while the menu is closed.
        if let Some(w) = windows.iter().next() {
            let size = (w.physical_width(), w.physical_height());
            let screen = pause_state::screen_size(size, size);
            if menu.prepared_for != Some(screen) {
                let _ = menu.commands.send(Command::Prepare { screen });
                menu.prepared_for = Some(screen);
            }
        }
    }
    // The movie draws its own cursor; the OS cursor is released and hidden
    // while the pause state is open (whether the game hides it is not traced).
    if menu.open != paused.0 {
        crate::platform::console::shared().write(|c| if menu.open { pause_state::enter(&mut c.cvars) } else { pause_state::exit(&mut c.cvars) });
        for mut c in &mut cursors {
            if menu.open {
                c.grab_mode = CursorGrabMode::None;
                c.visible = false;
            } else {
                c.visible = true;
            }
        }
    }
    paused.0 = menu.open;
}

/// Run condition for gameplay input: the pause decl blocks player input
/// (`m_blockPlayerInput`, binding set EMPTY) while the pause state is active.
pub fn game_not_paused(paused: Res<GamePaused>) -> bool {
    !paused.0
}

/// Diagnostic: `DH2_DEBUG_PAUSE_TIMING=1` logs, once a second, how long a
/// frame's tick waited in the movie thread's queue and how long it took.
mod debug_timing {
    use std::time::{Duration, Instant};

    #[derive(Default)]
    pub struct Timing {
        since: Option<Instant>,
        ticks: u32,
        waited: Duration,
        waited_max: Duration,
        work: Duration,
        work_max: Duration,
        /// Input, player tick, options and notifications, draw list.
        phases: [Duration; 4],
    }

    impl Timing {
        pub fn record(&mut self, waited: Duration, work: Duration, phases: [Duration; 4]) {
            if std::env::var_os("DH2_DEBUG_PAUSE_TIMING").is_none_or(|v| v != "1") {
                return;
            }
            let since = *self.since.get_or_insert_with(Instant::now);
            self.ticks += 1;
            self.waited += waited;
            self.waited_max = self.waited_max.max(waited);
            self.work += work;
            self.work_max = self.work_max.max(work);
            for (sum, p) in self.phases.iter_mut().zip(phases) {
                *sum += p;
            }
            if since.elapsed() >= Duration::from_secs(1) {
                let n = self.ticks.max(1);
                bevy::log::info!(
                    "Pause timing: {} ticks/s, queue wait avg {:.1} ms max {:.1} ms, work avg {:.1} ms max {:.1} ms (input {:.1}, tick {:.1}, options {:.1}, draw list {:.1})",
                    self.ticks,
                    self.waited.as_secs_f64() * 1000.0 / n as f64,
                    self.waited_max.as_secs_f64() * 1000.0,
                    self.work.as_secs_f64() * 1000.0 / n as f64,
                    self.work_max.as_secs_f64() * 1000.0,
                    self.phases[0].as_secs_f64() * 1000.0 / n as f64,
                    self.phases[1].as_secs_f64() * 1000.0 / n as f64,
                    self.phases[2].as_secs_f64() * 1000.0 / n as f64,
                    self.phases[3].as_secs_f64() * 1000.0 / n as f64
                );
                *self = Timing { since: Some(Instant::now()), ..Default::default() };
            }
        }
    }
}
