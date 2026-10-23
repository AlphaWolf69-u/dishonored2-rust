//! The pause state in the client: the game's Iggy pause movie, opened by the
//! keys bound to PAUSE and closed by the movie's `ResumeGame`.
//!
//! Host integration: the movie runs on its own thread with a large stack (the
//! interpreter nests AS3 calls on the native stack); the main thread sends it
//! input and frame ticks and receives draw lists.
use crate::assets::{archive::Catalog, installation, lang};
use crate::flash::{player::CharRef, raster, render, Player};
use crate::gameplay::game_clock_runtime::GamePaused;
use crate::gameplay::pause_state::{self, Character, Request};
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
    Tick(f64),
    /// OnExit: the movie is unloaded (`m_cacheResources` is 0).
    Close,
}

enum Event {
    Frame(FlashFrame),
    Resume,
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
    images: Arc<HashMap<CharRef, Arc<Image>>>,
    pending: f64,
    frame_interval: f64,
}

/// Main-thread owner of the pause state.
pub struct PauseMenu {
    commands: Sender<Command>,
    events: Receiver<Event>,
    /// Engine keys bound to PAUSE in the IN_GAME set.
    pause_keys: Vec<u32>,
    open: bool,
}

impl Default for PauseMenu {
    fn default() -> Self {
        let (commands, inbox) = channel();
        let (outbox, events) = channel();
        std::thread::Builder::new()
            .name("pause movie".into())
            .stack_size(256 << 20)
            .spawn(move || run(inbox, outbox))
            .expect("pause movie thread");
        let pause_keys = match installation::game_base()
            .map_err(|e| e.to_string())
            .and_then(|base| bindings::game_bindings(&base, user_config().as_deref()))
        {
            Ok(b) => b.keys_for_action(bindings::IN_GAME, PAUSE_ACTION),
            Err(e) => {
                error!("Pause bindings unavailable: {e}");
                Vec::new()
            }
        };
        Self { commands, events, pause_keys, open: false }
    }
}

fn user_config() -> Option<String> {
    game_settings::configuration_path().ok().and_then(|p| std::fs::read_to_string(p).ok())
}

/// The player character. The rewrite has no player/save state yet, so the
/// character cannot come from the game; `DH2_DEBUG_CHARACTER=emily` selects
/// Emily's movie for testing, Corvo otherwise.
fn debug_character() -> Character {
    match std::env::var("DH2_DEBUG_CHARACTER").as_deref() {
        Ok("emily") => Character::Emily,
        _ => Character::Corvo,
    }
}

fn load(screen: (i32, i32)) -> Result<Loaded, String> {
    let started = std::time::Instant::now();
    let base = installation::game_base().map_err(|e| e.to_string())?;
    let catalog = Catalog::open(&base).map_err(|e| e.to_string())?;
    let language = lang::read_text_language(&base)?;
    let movie = pause_state::movie(debug_character());
    let (mut player, resources) = Player::load_installed(&catalog, language, movie)?;
    pause_state::file_ready(&mut player, screen)?;
    let state = pause_state::current_state(&player, &language);
    let config = game_settings::configuration_path().ok();
    let mut pause = pause_state::Pause::new(&mut player, &catalog, &base, user_config().as_deref(), state, config)?;
    pause.on_flash_loaded(&mut player)?;
    pause.show(&mut player)?;
    let mut decoded = raster::Images::default();
    for (index, path) in &resources {
        let file = player.state.borrow().file(*index);
        decoded.load(&catalog, *index, path, &file)?;
    }
    let images = decoded
        .images
        .into_iter()
        .map(|(c, (width, height, rgba))| (c, Arc::new(Image { width, height, rgba })))
        .collect();
    let frame_interval = 1.0 / player.state.borrow().frame_rate;
    info!("Pause movie {movie} ({language}) loaded in {:.2} s", started.elapsed().as_secs_f64());
    Ok(Loaded { player, pause, cursor: (1e30, 1e30), pressed: None, images: Arc::new(images), pending: 0.0, frame_interval })
}

fn report_uncaught(player: &mut Player) {
    let uncaught = std::mem::take(&mut player.state.borrow_mut().uncaught);
    for e in uncaught {
        warn!("Pause movie: {e}");
    }
}

/// The movie thread: one player at a time, rebuilt on every Open.
fn run(inbox: Receiver<Command>, outbox: Sender<Event>) {
    let mut loaded: Option<Loaded> = None;
    let keyboard = Keyboard::default();
    let mut generation = 0u64;
    let mut unhandled = Vec::new();
    for command in inbox {
        match command {
            Command::Open { screen } => {
                generation += 1;
                match load(screen) {
                    Ok(l) => loaded = Some(l),
                    Err(e) => {
                        let _ = outbox.send(Event::Failed(e));
                    }
                }
            }
            Command::Close => loaded = None,
            Command::Key { down, key } => {
                let Some(l) = loaded.as_mut() else { continue };
                if down && !l.pressed.is_some_and(|k| k < 0xe0) {
                    l.pressed = Some(key);
                }
                // `FUN_140033740`: engine keys as virtual keys (`FUN_140032f20`), keyLocation 0.
                let vk = keyboard.virtual_key(key).unwrap_or(key);
                if let Err(e) = l.player.key_event(down, vk, 0) {
                    warn!("Pause movie key: {}", l.player.describe(&e));
                }
                report_uncaught(&mut l.player);
            }
            Command::Cursor { x, y } => {
                let Some(l) = loaded.as_mut() else { continue };
                // Moves of more than 0.1 movie pixel move `mouseCursor` and send
                // the Iggy move event.
                if (l.cursor.0 - x).abs() > 0.1 || (l.cursor.1 - y).abs() > 0.1 {
                    l.cursor = (x, y);
                    let moved = l
                        .player
                        .set_value("mouseCursor.x", x.into())
                        .and_then(|_| l.player.set_value("mouseCursor.y", y.into()))
                        .and_then(|_| l.player.mouse_move(x, y));
                    if let Err(e) = moved {
                        warn!("Pause movie cursor: {}", l.player.describe(&e));
                    }
                    report_uncaught(&mut l.player);
                }
            }
            Command::MouseButton { code, down } => {
                let Some(l) = loaded.as_mut() else { continue };
                if down && l.pressed.is_none() {
                    l.pressed = Some(code);
                }
                // The Iggy button event first (left/right/middle), then
                // `IG_OnMouseEvent(code, state)` for every button.
                let mut result = Ok(());
                if (0xe0..=0xe2).contains(&code) {
                    result = l.player.mouse_button((code - 0xe0) as u8, down);
                }
                let state = if down { 1.0 } else { 0.0 };
                let result = result.and_then(|_| {
                    l.player.call_path_method("inputsManager.mouseHandler", "IG_OnMouseEvent", &[(code as f64).into(), state.into()]).map(|_| ())
                });
                if let Err(e) = result {
                    warn!("Pause movie mouse: {}", l.player.describe(&e));
                }
                report_uncaught(&mut l.player);
            }
            Command::Wheel(delta) => {
                let Some(l) = loaded.as_mut() else { continue };
                if let Err(e) = l.player.mouse_wheel(delta) {
                    warn!("Pause movie wheel: {}", l.player.describe(&e));
                }
                report_uncaught(&mut l.player);
            }
            Command::Tick(dt) => {
                let Some(l) = loaded.as_mut() else { continue };
                // `IggyPlayerReadyToTick`/`FUN_141021b40`: one tick per frame interval.
                l.pending += dt;
                if l.pending >= l.frame_interval {
                    let step = l.pending.min(l.frame_interval);
                    l.pending = (l.pending - l.frame_interval).max(0.0);
                    if let Err(e) = l.player.tick(step) {
                        warn!("Pause movie tick: {}", l.player.describe(&e));
                    }
                    report_uncaught(&mut l.player);
                }
                let pressed = l.pressed.take();
                if let Err(e) = l.pause.update(&mut l.player, pressed) {
                    warn!("Pause options: {e}");
                }
                match l.pause.handle_notifications(&mut l.player, &mut unhandled) {
                    Ok(requests) => {
                        for request in requests {
                            match request {
                                Request::Resume => {
                                    let _ = outbox.send(Event::Resume);
                                }
                                // The message-box host (`global.iggy`) is not ported yet.
                                Request::MessageBox(b) => warn!("Pause message box not shown yet: {}", b.text),
                                // The renderer mode switch is not wired to the window yet.
                                Request::DisplayChanged => warn!("Pause display settings changed; applied on the next start"),
                            }
                        }
                    }
                    Err(e) => warn!("Pause movie: {e}"),
                }
                for name in unhandled.drain(..) {
                    debug!("Pause movie notification not handled yet: {name}");
                }
                // Diagnostic: DH2_DEBUG_FLASH_ALL=1 also draws hidden objects.
                let debug_all = std::env::var_os("DH2_DEBUG_FLASH_ALL").is_some_and(|v| v == "1");
                let list = if debug_all { render::debug_draw_all(&l.player.vm) } else { render::draw_list(&l.player.vm) };
                let _ = outbox.send(Event::Frame(FlashFrame { list: Arc::new(list), images: l.images.clone(), generation }));
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
) {
    let menu = &mut *menu;
    for input in keys.read() {
        // Engine keyboard events do not repeat.
        if input.repeat {
            continue;
        }
        let Some(key) = super::engine_keys::engine_key(input.key_code) else { continue };
        let down = input.state == ButtonState::Pressed;
        if menu.open {
            let _ = menu.commands.send(Command::Key { down, key });
        } else if down && menu.pause_keys.contains(&key) {
            // `SwitchState(GAME_LOGIC_STATE_PAUSE)`; the UI renders at window size.
            let size = windows.iter().next().map_or((1920, 1080), |w| (w.physical_width(), w.physical_height()));
            let screen = pause_state::screen_size(size, size);
            let _ = menu.commands.send(Command::Open { screen });
            menu.open = true;
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
        let _ = menu.commands.send(Command::Tick(time.delta_secs_f64()));
    } else {
        buttons.clear();
        wheels.clear();
    }
    for event in menu.events.try_iter() {
        match event {
            Event::Frame(f) if menu.open => frame.0 = Some(f),
            Event::Frame(_) => {}
            Event::Resume => {
                // `FUN_14078ca40` pops the state; OnExit unloads the movie.
                let _ = menu.commands.send(Command::Close);
                menu.open = false;
            }
            Event::Failed(e) => {
                error!("Pause menu unavailable: {e}");
                menu.open = false;
            }
        }
    }
    if !menu.open {
        frame.0 = None;
    }
    // The movie draws its own cursor; the OS cursor is released and hidden
    // while the pause state is open (whether the game hides it is not traced).
    if menu.open != paused.0 {
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
