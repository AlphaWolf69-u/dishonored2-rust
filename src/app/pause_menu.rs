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
use bevy::input::{keyboard::KeyboardInput, ButtonState};
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
    let state = pause_state::GameState::default();
    pause_state::on_flash_loaded(&mut player, &base, user_config().as_deref(), &mut Keyboard::default(), &state)?;
    pause_state::show(&mut player, &state)?;
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
    Ok(Loaded { player, images: Arc::new(images), pending: 0.0, frame_interval })
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
                // `FUN_140033740`: engine keys as virtual keys (`FUN_140032f20`), keyLocation 0.
                let vk = keyboard.virtual_key(key).unwrap_or(key);
                if let Err(e) = l.player.key_event(down, vk, 0) {
                    warn!("Pause movie key: {}", l.player.describe(&e));
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
                match pause_state::handle_notifications(&mut l.player, &mut unhandled) {
                    Ok(requests) => {
                        if requests.contains(&Request::Resume) {
                            let _ = outbox.send(Event::Resume);
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
pub fn update_pause_menu(
    mut menu: NonSendMut<PauseMenu>,
    mut keys: MessageReader<KeyboardInput>,
    time: Res<Time>,
    windows: Query<&Window>,
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
        let _ = menu.commands.send(Command::Tick(time.delta_secs_f64()));
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
    paused.0 = menu.open;
}

/// Run condition for gameplay input: the pause decl blocks player input
/// (`m_blockPlayerInput`, binding set EMPTY) while the pause state is active.
pub fn game_not_paused(paused: Res<GamePaused>) -> bool {
    !paused.0
}
