//! The installed quick-access wheel on the existing Iggy worker.
//!4f6030 caches the wheelData bindings;4f6d50 supplies rows and calls Show.
use crate::{
    flash::{
        Player,
        avm2::value::{Value, to_int32},
        render,
    },
    gameplay::{
        pause_state,
        power_wheel::{Potions, Snapshot},
    },
    rendering::bevy_backend::flash_ui::FlashFrame,
};
use std::sync::{Arc, mpsc::Sender};

pub(super) struct Command {
    pub epoch: u64,
    pub operation: Operation,
}
pub(super) enum Operation {
    Prepare {
        screen: (i32, i32),
        path: String,
        events: Sender<Event>,
    },
    Open(Snapshot),
    Key {
        down: bool,
        key: u32,
    },
    Cursor {
        x: f64,
        y: f64,
    },
    Button {
        down: bool,
        button: u8,
    },
    Wheel(i32),
    Tick(f64),
    ForceExit,
    Refresh(Potions),
    Clear {
        events: Sender<Event>,
    },
}
pub(super) struct Event {
    pub epoch: u64,
    pub notification: Notification,
}
pub(super) enum Notification {
    Cleared,
    Prepared { interval: f64 },
    Frame(Option<FlashFrame>),
    Closing { selected: i32, visible: Vec<bool> },
    Disabled,
    Shortcut { slot: usize, index: i32 },
    HealthPotion,
    ManaPotion,
    Pause,
    Sound(u32),
    Failed(String),
}
#[derive(Default)]
pub(super) struct Worker {
    epoch: u64,
    events: Option<Sender<Event>>,
    movie: Option<Movie>,
}
impl Worker {
    pub fn clear(&mut self) {
        self.movie = None;
        self.events = None;
    }
    fn send(&self, notification: Notification) {
        if let Some(outbox) = &self.events {
            let _ = outbox.send(Event {
                epoch: self.epoch,
                notification,
            });
        }
    }
    pub fn command(&mut self, command: Command, generation: &mut u64) {
        if command.epoch != self.epoch {
            if !matches!(
                &command.operation,
                Operation::Prepare { .. } | Operation::Clear { .. }
            ) {
                return;
            }
            self.clear();
            self.epoch = command.epoch;
        }
        let result = (|| -> Result<(), String> {
            match command.operation {
                Operation::Prepare {
                    screen,
                    path,
                    events,
                } => {
                    self.clear();
                    self.events = Some(events);
                    *generation = generation.wrapping_add(1);
                    let movie = Movie::load(screen, *generation, &path)?;
                    let interval = movie.interval;
                    self.movie = Some(movie);
                    self.send(Notification::Prepared { interval });
                }
                Operation::Clear { events } => {
                    self.clear();
                    let _ = events.send(Event {
                        epoch: self.epoch,
                        notification: Notification::Cleared,
                    });
                }
                operation => {
                    let movie = self
                        .movie
                        .as_mut()
                        .ok_or("wheel command has no prepared movie")?;
                    match operation {
                        Operation::Open(snapshot) => movie.open(snapshot)?,
                        Operation::Refresh(snapshot) => movie.refresh(&snapshot)?,
                        Operation::Key { down, key } => movie.key(down, key)?,
                        Operation::Cursor { x, y } => movie.cursor(x, y)?,
                        Operation::Button { down, button } => movie.button(down, button)?,
                        Operation::Wheel(delta) => {
                            movie
                                .player
                                .mouse_wheel(delta)
                                .map_err(|error| movie.player.describe(&error))?;
                            movie.last = None;
                        }
                        Operation::ForceExit => {
                            movie
                                .player
                                .call_root_method("_C_ForceExit", &[])
                                .map_err(|error| movie.player.describe(&error))?;
                            movie.last = None;
                        }
                        Operation::Tick(dt) => {
                            let (frame, notifications) = movie.frame(dt)?;
                            for notification in notifications {
                                self.send(notification);
                            }
                            self.send(Notification::Frame(frame));
                        }
                        Operation::Prepare { .. } | Operation::Clear { .. } => unreachable!(),
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.send(Notification::Failed(error));
            self.clear();
        }
    }
}

struct Movie {
    player: Player,
    images: Arc<crate::flash::bitmap_resources::MovieBitmaps>,
    cache: render::Cache,
    generation: u64,
    serial: u64,
    accumulated: f64,
    elapsed: f64,
    interval: f64,
    last: Option<FlashFrame>,
    snapshot: Option<Snapshot>,
    focused: bool,
    _residency: Option<super::pause_menu::MovieResidency>,
}
impl Movie {
    fn load(screen: (i32, i32), generation: u64, state_path: &str) -> Result<Self, String> {
        let catalog = super::pause_menu::installed_catalog()?;
        let language = crate::assets::lang::read_text_language(catalog.base())?;
        let (resource, descriptor) =
            super::pause_menu::state_movie_info(&catalog, "gamelogicstatepowerwheel", state_path)?;
        let (mut player, resources) = Player::load_installed(&catalog, language, &resource)?;
        pause_state::file_ready(&mut player, screen)?;
        let strings = player
            .state
            .borrow()
            .strings
            .clone()
            .ok_or("wheel string table absent")?;
        let actions = crate::platform::bindings::bindable_actions(
            &crate::platform::console::shared().read().bindings,
            &strings,
            &mut crate::platform::bindings::Keyboard::default(),
        );
        pause_state::write_bindable_actions(&mut player, &actions)
            .map_err(|error| player.describe(&error))?;
        let images = super::pause_menu::library_images(
            &catalog,
            &player,
            &resources,
            crate::flash::bitmap_resources::PackLifetime::Session,
        )?;
        let residency =
            super::pause_menu::MovieResidency::new("power-wheel", &catalog, &resources, &images);
        let stage = player.state.borrow().stage_size;
        let interval = 1. / player.state.borrow().frame_rate;
        let mut cache = render::Cache::default();
        cache.set_font_caching(pause_state::font_caching_fields(
            &descriptor,
            &resource,
            stage,
            (screen.0 as u32, screen.1 as u32),
        ));
        super::pause_menu::report_uncaught(&mut player, "Power wheel");
        Ok(Self {
            player,
            images: Arc::new(images),
            cache,
            generation,
            serial: 0,
            accumulated: 0.,
            elapsed: 0.,
            interval,
            last: None,
            snapshot: None,
            focused: false,
            _residency: residency,
        })
    }
    fn refresh(&mut self, snapshot: &Potions) -> Result<(), String> {
        let apply = (|| -> crate::flash::avm2::Result<()> {
            self.player.set_value(
                "wheelData.canConsumeHealthPotion",
                snapshot.can_health.into(),
            )?;
            self.player
                .set_value("wheelData.canConsumeManaPotion", snapshot.can_mana.into())?;
            self.player
                .set_value("wheelData.healthPotionCount", snapshot.health_count.into())?;
            self.player
                .set_value("wheelData.manaPotionCount", snapshot.mana_count.into())?;
            Ok(())
        })();
        apply.map_err(|error| self.player.describe(&error))?;
        self.last = None;
        Ok(())
    }
    fn open(&mut self, snapshot: Snapshot) -> Result<(), String> {
        let strings = self
            .player
            .state
            .borrow()
            .strings
            .clone()
            .ok_or("wheel string table absent")?;
        let configure = (|| -> crate::flash::avm2::Result<()> {
            self.player.set_value(
                "wheelData.equipmentData.length",
                (snapshot.items.len() as u32).into(),
            )?;
            for (index, item) in snapshot.items.iter().enumerate() {
                let row = format!("wheelData.equipmentData.data[{index}]");
                for (name, value) in [
                    ("label", strings.lookup(item.label()).into()),
                    ("description", strings.lookup(item.description()).into()),
                    ("itemIndex", item.index().into()),
                    ("inventoryIndex", item.index().into()),
                    ("ammoCount", item.count.into()),
                    ("ammoMax", item.maximum.into()),
                    ("visible", item.visible.into()),
                ] {
                    self.player.set_value(&format!("{row}.{name}"), value)?;
                }
            }
            for (provider, range) in [("shortcutPCData", 0..10), ("shortcutDPadData", 10..14)] {
                let values = &snapshot.shortcuts[range];
                // WheelGameData constructs plain Arrays (10 PC / 4 DPad),
                // unlike equipmentData's GameDataProvider. 4f6c20 iterates
                // their elements directly through0280b0, with no `data` member.
                self.player.set_value(
                    &format!("wheelData.{provider}.length"),
                    (values.len() as u32).into(),
                )?;
                for (index, &value) in values.iter().enumerate() {
                    self.player
                        .set_value(&format!("wheelData.{provider}[{index}]"), value.into())?;
                }
            }
            self.player.call_path_method(
                "controllerTypeHandler",
                "_C_UpdateControllerType",
                &[pause_state::controller_type().into()],
            )?;
            Ok(())
        })();
        configure.map_err(|error| self.player.describe(&error))?;
        self.player
            .set_value(
                "wheelData.currentEquippedItemIndex",
                snapshot.current.into(),
            )
            .map_err(|error| self.player.describe(&error))?;
        self.refresh(&snapshot.potions())?;
        //4f5820 unions fixed engine keys then maps through032f20.
        let binding = |action, extra: &[u32], close: bool| -> Vec<Value> {
            let mut keys = crate::platform::console::shared()
                .read()
                .bindings
                .keys_for_action(crate::platform::bindings::IN_GAME, action);
            for key in extra {
                if !keys.contains(key) {
                    keys.push(*key);
                }
            }
            if close && !keys.is_empty() && !keys.contains(&0) {
                keys.push(0);
            }
            let keyboard = crate::platform::bindings::Keyboard::default();
            keys.into_iter()
                .map(|key| keyboard.virtual_key(key).unwrap_or(key).into())
                .collect()
        };
        let close = self.player.array(binding(45, &[], true));
        let health = self.player.array(binding(35, &[0x13, 0xf2], false));
        let mana = self.player.array(binding(36, &[0x14, 0xf3], false));
        self.player
            .call_root_method("_C_Show", &[close, health, mana])
            .map_err(|error| self.player.describe(&error))?;
        self.snapshot = Some(snapshot);
        self.focused = true;
        self.accumulated = 0.;
        self.elapsed = 0.;
        self.last = None;
        Ok(())
    }
    fn key(&mut self, down: bool, key: u32) -> Result<(), String> {
        if !self.focused {
            return Ok(());
        }
        let key = crate::platform::bindings::Keyboard::default()
            .virtual_key(key)
            .unwrap_or(key);
        self.player
            .key_event(down, key, 0)
            .map_err(|error| self.player.describe(&error))?;
        self.last = None;
        Ok(())
    }
    fn cursor(&mut self, x: f64, y: f64) -> Result<(), String> {
        if !self.focused {
            return Ok(());
        }
        self.player
            .set_value("mouseCursor.x", x.into())
            .and_then(|_| self.player.set_value("mouseCursor.y", y.into()))
            .and_then(|_| self.player.mouse_move(x, y))
            .map_err(|error| self.player.describe(&error))?;
        self.last = None;
        Ok(())
    }
    fn button(&mut self, down: bool, button: u8) -> Result<(), String> {
        if !self.focused {
            return Ok(());
        }
        if button < 3 {
            self.player
                .mouse_button(button, down)
                .map_err(|error| self.player.describe(&error))?;
        }
        self.player
            .call_path_method(
                "inputsManager.mouseHandler",
                "IG_OnMouseEvent",
                &[
                    (0xe0 + button as u32).into(),
                    (if down { 1. } else { 0. }).into(),
                ],
            )
            .map_err(|error| self.player.describe(&error))?;
        self.last = None;
        Ok(())
    }
    fn frame(&mut self, dt: f64) -> Result<(Option<FlashFrame>, Vec<Notification>), String> {
        self.accumulated += dt;
        self.elapsed += dt;
        let tick = self.accumulated >= self.interval;
        let changed = tick || self.last.is_none();
        if tick {
            let step = std::mem::take(&mut self.elapsed);
            self.accumulated = (self.accumulated - self.interval).max(0.);
            self.player
                .tick(step)
                .map_err(|error| self.player.describe(&error))?;
        }
        let mut events = Vec::new();
        for (name, args) in self.player.take_notifications() {
            match name.as_str() {
                "WheelClosing" => {
                    let selected = self
                        .player
                        .get_value("wheelData.selectedInventoryIndex")
                        .map(|value| to_int32(value.primitive_to_number()))
                        .map_err(|error| self.player.describe(&error))?;
                    let count = self
                        .snapshot
                        .as_ref()
                        .map_or(0, |snapshot| snapshot.items.len());
                    let mut visible = Vec::with_capacity(count);
                    for index in 0..count {
                        visible.push(
                            self.player
                                .get_value(&format!(
                                    "wheelData.equipmentData.data[{index}].visible"
                                ))
                                .map_err(|error| self.player.describe(&error))?
                                .to_boolean(),
                        );
                    }
                    events.push(Notification::Closing { selected, visible });
                }
                "OnWheelDisabled" => {
                    self.focused = false;
                    self.snapshot = None;
                    events.push(Notification::Disabled);
                }
                "ConsumeHealthPotion" => events.push(Notification::HealthPotion),
                "ConsumeManaPotion" => events.push(Notification::ManaPotion),
                "OnPauseRequest" => events.push(Notification::Pause),
                "OnShortcutDPadChanged" | "OnShortcutKeyboardChanged" => {
                    if args.len() != 2 {
                        continue;
                    }
                    if let (Some(slot), Some(index)) = (args.first(), args.get(1)) {
                        let offset = if name == "OnShortcutDPadChanged" {
                            10
                        } else {
                            0
                        };
                        let slot = to_int32(slot.primitive_to_number());
                        if slot >= 0 {
                            events.push(Notification::Shortcut {
                                slot: slot as usize + offset,
                                index: to_int32(index.primitive_to_number()),
                            });
                        }
                    }
                }
                "SetFlashFocus" => self.focused = true,
                "KillFlashFocus" => self.focused = false,
                "PostSoundEvent" => {
                    if let Some(value) = args.first() {
                        events.push(Notification::Sound(crate::flash::avm2::value::to_uint32(
                            value.primitive_to_number(),
                        )));
                    }
                }
                _ => super::pause_menu::report_unknown_callback("Power wheel", &name),
            }
        }
        if changed {
            self.serial = self.serial.wrapping_add(1);
            self.last = Some(FlashFrame {
                list: Arc::new(render::draw_list_cached(&self.player.vm, &mut self.cache)),
                images: self.images.clone(),
                game_images: Default::default(),
                generation: self.generation,
                serial: self.serial,
            });
        }
        super::pause_menu::report_uncaught(&mut self.player, "Power wheel");
        Ok((changed.then(|| self.last.clone()).flatten(), events))
    }
}
