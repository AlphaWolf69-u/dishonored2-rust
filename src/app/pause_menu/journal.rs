//! Journal/readable states hosted by the existing UI worker and player runtime.
use super::*;
use crate::gameplay::{game_logic::{GameLogic, StateId}, player::read_note::Presentation};
use super::super::{interaction_focus::{DiscoveredDocuments, DocumentEntry, JournalItem}, journal_movie, readable_movie};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind { Journal, Readable }
pub(super) struct Snapshot {
    documents: Vec<DocumentEntry>, maps: Vec<JournalItem>, audiographs: Vec<JournalItem>,
    progress: Option<crate::gameplay::gameflow::State>,
    playing_audiograph: Option<u32>,
    powers: Option<journal_movie::PowerData>,
}
pub(super) struct Command { pub epoch: u64, pub operation: Operation }
pub(super) enum Operation {
    Journal { screen: (i32,i32), path: String, snapshot: Snapshot },
    Readable { screen: (i32,i32), path: String, presentation: Arc<Presentation> },
    Key { down: bool, key: u32 }, Cursor { x: f64, y: f64 }, Button { down: bool, button: u8 }, Wheel(i32),
    Audiograph(Option<u32>), PurchaseResult { success: bool, runes: u32 }, Tick(f64), Clear,
}
pub(super) struct Event { pub epoch: u64, pub payload: Notification }
pub(super) enum Notification {
    Ready(Kind), Frame(Option<FlashFrame>), Closed(Kind), Failed(String),
    Actions(Vec<journal_movie::Action>), Presentation(Arc<Presentation>),
    ScreenFx(bool),
}
#[derive(Default)]
pub(super) struct Worker {
    epoch: u64, journal: Option<journal_movie::Movie>, readable: Option<readable_movie::Movie>,
}
impl Worker {
    pub fn clear(&mut self) { self.readable = None; self.journal = None; }
    pub fn command(&mut self, command: Command, generation: &mut u64, outbox: &Sender<super::Event>) {
        if command.epoch != self.epoch {
            if !matches!(&command.operation, Operation::Journal {..} | Operation::Readable {..} | Operation::Clear) { return; }
            self.clear(); self.epoch = command.epoch;
        }
        let send = |payload| { let _ = outbox.send(super::Event::Modal(Event { epoch: command.epoch, payload })); };
        let result: Result<(), String> = (|| {
            match command.operation {
                Operation::Journal { screen, path, snapshot } => {
                    self.clear(); *generation = generation.wrapping_add(1);
                    trace_readable_ui(|| format!("Journal movie loading epoch={} path={path} documents={} maps={} audiographs={}", command.epoch, snapshot.documents.len(), snapshot.maps.len(), snapshot.audiographs.len()));
                    self.journal = Some(journal_movie::Movie::load(screen, *generation, &path,
                        snapshot.documents, snapshot.maps, snapshot.audiographs, snapshot.playing_audiograph, snapshot.powers, snapshot.progress)?);
                    send(Notification::Ready(Kind::Journal));
                },
                Operation::Readable { screen, path, presentation } => {
                    *generation = generation.wrapping_add(1);
                    trace_readable_ui(|| format!("Readable movie loading epoch={} path={path}", command.epoch));
                    self.readable = Some(readable_movie::Movie::load(screen, &presentation, *generation, &path)?);
                    send(Notification::Ready(Kind::Readable));
                },
                Operation::Key { down, key } => {
                    let vk = Keyboard::default().virtual_key(key).unwrap_or(key);
                    if let Some(movie) = &mut self.readable { movie.key(down, vk)?; }
                    else if let Some(movie) = &mut self.journal { movie.key(down, vk)?; }
                },
                Operation::Cursor { x, y } => {
                    if let Some(movie) = &mut self.readable { movie.cursor(x,y)?; }
                    else if let Some(movie) = &mut self.journal { movie.cursor(x,y)?; }
                },
                Operation::Button { down, button } => {
                    if let Some(movie) = &mut self.readable { movie.mouse_button(down, button)?; }
                    else if let Some(movie) = &mut self.journal { movie.button(down, button)?; }
                },
                Operation::Wheel(delta) => {
                    if let Some(movie) = &mut self.readable { movie.mouse_wheel(delta)?; }
                    else if let Some(movie) = &mut self.journal { movie.wheel(delta)?; }
                },
                Operation::Tick(dt) => {
                    if let Some(movie) = &mut self.readable {
                        let frame = movie.frame(dt, false)?;
                        for id in frame.sounds { let _ = outbox.send(super::Event::ModalSound(command.epoch, id)); }
                        if let Some(on) = frame.screen_fx { send(Notification::ScreenFx(on)); }
                        send(Notification::Frame(frame.frame));
                        if frame.closed {
                            self.readable = None;
                            if let Some(journal) = &mut self.journal { journal.subscreen_closed()?; }
                            send(Notification::Closed(Kind::Readable));
                        }
                    } else if let Some(movie) = &mut self.journal {
                        let (frame, closed, sounds, actions) = movie.frame(dt, false)?;
                        for id in sounds { let _ = outbox.send(super::Event::ModalSound(command.epoch, id)); }
                        send(Notification::Frame(frame));
                        if !actions.is_empty() {
                            for action in &actions {
                                if let journal_movie::Action::Show(path) | journal_movie::Action::ShowMap(path) = action {
                                    let item = movie.item(path).ok_or("Journal item lease is absent")?;
                                    send(Notification::Presentation(presentation(item)?));
                                }
                            }
                            send(Notification::Actions(actions));
                        }
                        if closed { self.journal = None; send(Notification::Closed(Kind::Journal)); }
                    }
                },
                Operation::Audiograph(playing) => {
                    if let Some(movie) = &mut self.journal { movie.set_playing_audiograph(playing)?; }
                },
                Operation::PurchaseResult { success, runes } => {
                    if let Some(movie) = &mut self.journal { movie.purchase_result(success, runes)?; }
                },
                Operation::Clear => self.clear(),
            }
            Ok(())
        })();
        // Lifecycle/event callbacks store uncaught AS3 exceptions as Iggy
        // does; drain the same reporter as Pause after every operation.
        if let Some(movie) = &mut self.readable { movie.report_uncaught(); }
        if let Some(movie) = &mut self.journal { movie.report_uncaught(); }
        if let Err(error) = result { self.clear(); send(Notification::Failed(error)); }
    }
}
pub(in crate::app) fn presentation(item: &crate::assets::inventory_item::PickableItem) -> Result<Arc<Presentation>, String> {
    use crate::assets::inventory_item::PickableReadable;
    Ok(Arc::new(match &item.readable {
        Some(PickableReadable::Note(note) | PickableReadable::Other(note, _)) => Presentation::Note {
            title: note.title.clone(), text: note.text.clone(), note_type: note.note_type },
        Some(PickableReadable::Map(map)) => Presentation::Map {
            title: map.title.clone(), image: map.image.clone().unwrap_or_default(), legends: map.legends.clone(), map_type: map.map_type },
        None => return Err("Journal item has no supported reading presentation".into()),
    }))
}

#[derive(Default)]
pub(super) struct Session { pub active: Option<Kind>, pub ready: bool, pub in_flight: bool, hud_ready: bool, read_admission: Option<bool>, elapsed: f64, cursor: Option<(f64,f64)>, playing_audiograph: Option<u32> }
#[derive(bevy::ecs::system::SystemParam)]
pub struct Data<'w> {
    logic: Option<ResMut<'w, GameLogic>>,
    audio: ResMut<'w, super::super::audio::audiographs::Audiographs>,
    loading: Res<'w, super::super::loading_screen::Acquisition>,
    progress: Option<ResMut<'w, crate::gameplay::gameflow::State>>,
    requests: Option<ResMut<'w, readable_movie::ReadableRequests>>,
    powers: Option<Res<'w, super::super::powers::Powers>>,
}
impl Data<'_> {
    pub(super) fn tasks_notified(&mut self,ids:Vec<u32>){if let Some(progress)=self.progress.as_deref_mut(){for id in ids{progress.mark_task_notified(id);}}}
}
impl Session {
    pub fn hud_ready(&mut self) { self.hud_ready = true; }
    pub fn clear(&mut self) { *self = Self::default(); }
    fn send(&self, epoch: u64, operation: Operation, commands: &Sender<super::Command>) {
        if let Err(error) = commands.send(super::Command::Modal(Command { epoch, operation })) {
            warn!("Journal/reading request could not reach its worker: {error}");
        }
    }
    pub fn input(&mut self, epoch: u64, operation: Operation, commands: &Sender<super::Command>) {
        if let Operation::Cursor { x, y } = &operation {
            if self.cursor == Some((*x,*y)) { return; }
            self.cursor = Some((*x,*y));
        }
        self.send(epoch,operation,commands);
    }
    pub fn begin_pause(&mut self, data: &mut Data) -> Result<(), String> {
        if let Some(logic) = data.logic.as_deref_mut() {
            if !logic.request(StateId::PAUSE) { return Err("pause state request is unavailable".into()); }
            logic.commit()?.ok_or("pause state transition was not committed")?;
        }
        Ok(())
    }
    pub fn end_pause(&mut self, data: &mut Data) {
        if let Some(logic) = data.logic.as_deref_mut() {
            if logic.current() == Some(StateId::PAUSE) { logic.request_finish_current(); logic.commit_finish(); }
        }
    }
    fn finish_modals(&mut self, data: &mut Data) {
        if let Some(logic) = data.logic.as_deref_mut() {
            while matches!(logic.current(), Some(StateId::JOURNAL | StateId::READ_NOTE)) {
                logic.request_finish_current(); logic.commit_finish();
            }
        }
    }
    fn switch(&mut self, target: StateId, data: &mut Data) -> Result<String, String> {
        let logic = data.logic.as_deref_mut().ok_or("Journal game-logic owner is absent")?;
        let path = logic.declaration.states[target.0 as usize].clone().ok_or("Journal state declaration is absent")?;
        if !logic.request(target) { return Err("authored game-logic rules reject the requested UI state".into()); }
        let transition = logic.commit()?.ok_or("Journal state transition was not committed")?;
        if transition.entered != target { return Err("Journal state transition identity differs".into()); }
        self.active = Some(if target == StateId::JOURNAL { Kind::Journal } else { Kind::Readable });
        self.ready = false; self.in_flight = false; self.elapsed = 0.0; self.cursor = None;
        Ok(path)
    }
    pub fn initialize(&mut self, data: &mut Data) -> Result<(), String> {
        if !self.hud_ready { return Ok(()); }
        let Some(logic) = data.logic.as_deref_mut() else { return Ok(()) };
        if matches!(logic.current(), None | Some(StateId::MAP_TRANSITION)) { logic.enter_initial_state()?; }
        Ok(())
    }
    pub fn open_journal(&mut self, epoch: u64, screen: (i32,i32), data: &mut Data,
        inventory: Option<&DiscoveredDocuments>, commands: &Sender<super::Command>) -> Result<(), String> {
        trace_readable_ui(|| format!("Journal entry epoch={epoch} hud_ready={} active={:?} logic={:?} journal_decl={}", self.hud_ready, self.active,
            data.logic.as_deref().and_then(|logic| logic.current()), data.logic.as_deref().is_some_and(|logic| logic.available(StateId::JOURNAL))));
        if self.active.is_some() { return Ok(()); }
        if !self.hud_ready { return Err("HUD has not completed initialization".into()); }
        let inventory = inventory.ok_or("Journal inventory owner is absent")?;
        let progress = data.progress.as_deref().cloned();
        let snapshot = Snapshot { documents: inventory.items().to_vec(), maps: inventory.maps().to_vec(),
            audiographs: inventory.audiographs().to_vec(), progress, playing_audiograph: data.audio.playing(),
            powers: data.powers.as_deref().map(|powers| powers.snapshot(inventory)) };
        self.playing_audiograph = snapshot.playing_audiograph;
        let path = self.switch(StateId::JOURNAL, data)?;
        self.send(epoch, Operation::Journal { screen, path, snapshot }, commands);
        Ok(())
    }
    pub fn update(&mut self, epoch: u64, screen: (i32,i32), data: &mut Data,
        mut player: Option<&mut crate::gameplay::player::Player>, commands: &Sender<super::Command>, dt: f64, paused: bool) -> Result<(), String> {
        self.initialize(data)?;
        if self.active.is_some() && self.playing_audiograph != data.audio.playing() {
            self.playing_audiograph = data.audio.playing();
            self.send(epoch, Operation::Audiograph(self.playing_audiograph), commands);
        }
        if let Some(player) = player.as_deref_mut() {
            let available = self.hud_ready && !paused && self.active.is_none()
                && data.logic.as_deref().is_some_and(|logic| logic.available(StateId::READ_NOTE)
                    && logic.current().is_some_and(|current| logic.declaration.rule(current, StateId::READ_NOTE).is_some_and(|rule| rule.allow)));
            if self.read_admission != Some(available) {
                trace_readable_ui(|| format!("READ_NOTE availability={available} hud_ready={} paused={paused} active={:?} declaration={} current={:?}", self.hud_ready, self.active,
                    data.logic.as_deref().is_some_and(|logic| logic.available(StateId::READ_NOTE)), data.logic.as_deref().and_then(|logic| logic.current())));
                self.read_admission = Some(available);
            }
            player.set_read_note_admission(available);
        }
        if self.active != Some(Kind::Readable) {
            let request = data.requests.as_deref_mut().and_then(|requests| requests.open.pop_front());
            if let Some(presentation) = request {
                let path = match self.switch(StateId::READ_NOTE,data) {
                    Ok(path) => path,
                    Err(error) => {
                        if let Some(player) = player.as_deref_mut() { player.finish_read_note(); }
                        if let Some(requests) = data.requests.as_deref_mut() { requests.failed = requests.failed.wrapping_add(1); }
                        return Err(error);
                    }
                };
                self.send(epoch, Operation::Readable { screen, path, presentation }, commands);
            }
        }
        if self.active.is_some() { self.elapsed += dt; }
        Ok(())
    }
    pub fn tick(&mut self, epoch: u64, commands: &Sender<super::Command>) {
        if self.active.is_some() && self.ready && !self.in_flight {
            let elapsed = std::mem::take(&mut self.elapsed);
            self.send(epoch, Operation::Tick(elapsed), commands); self.in_flight = true;
        }
    }
    pub fn event(&mut self, epoch: u64, event: Event, screen: (i32,i32), data: &mut Data,
        inventory: Option<&mut DiscoveredDocuments>, player: Option<&mut crate::gameplay::player::Player>,
        frame: &mut FlashUiFrame, commands: &Sender<super::Command>) -> Result<(), String> {
        if event.epoch != epoch { return Ok(()); }
        match event.payload {
            Notification::Ready(kind) if self.active == Some(kind) => { trace_readable_ui(|| format!("Movie ready epoch={epoch} kind={kind:?}")); self.ready = true; },
            Notification::Ready(_) => {},
            Notification::ScreenFx(_) => {}, // Applied by the same epoch-checked UI owner.
            Notification::Frame(value) => { self.in_flight = false; if self.active.is_some() { if let Some(value) = value { frame.0 = Some(value); } } },
            Notification::Closed(kind) => {
                if let Some(logic) = data.logic.as_deref_mut() { logic.request_finish_current(); logic.commit_finish();
                    self.active = (logic.current() == Some(StateId::JOURNAL)).then_some(Kind::Journal); }
                else { self.active = None; }
                self.ready = self.active.is_some(); self.in_flight = false;
                if kind == Kind::Readable {
                    if let Some(requests) = data.requests.as_deref_mut() { requests.closed = requests.closed.wrapping_add(1); }
                    if let Some(player) = player { player.finish_read_note(); }
                }
                if self.active.is_none() { frame.0 = None; }
            },
            Notification::Failed(error) => {
                trace_readable_ui(|| format!("Movie failed epoch={epoch} active={:?}: {error}", self.active));
                self.active = None; self.ready = false; self.in_flight = false; frame.0 = None;
                if let Some(player) = player { player.finish_read_note(); }
                if let Some(requests) = data.requests.as_deref_mut() { requests.failed = requests.failed.wrapping_add(1); }
                self.finish_modals(data);
                return Err(error);
            },
            Notification::Presentation(presentation) => {
                let player = player.ok_or("Journal reading player is absent")?;
                if !player.set_read_note_presentation(presentation.clone()) { return Err("Journal READ_NOTE descriptor owner is absent".into()); }
                let path = match self.switch(StateId::READ_NOTE,data) {
                    Ok(path) => path,
                    Err(error) => { player.finish_read_note(); return Err(error); }
                };
                self.send(epoch, Operation::Readable { screen, path, presentation }, commands);
            },
            Notification::Actions(actions) => {
                let inventory = inventory.ok_or("Journal inventory owner is absent")?;
                for action in actions {
                    match action {
                        journal_movie::Action::Seen(kind,id) => inventory.mark_seen(kind,id),
                        journal_movie::Action::SeenMap(id) => inventory.mark_maps_seen(id),
                        journal_movie::Action::SeenAudiographs => inventory.mark_audiographs_seen(None),
                        journal_movie::Action::ShowHistory(show) => if let Some(progress) = data.progress.as_deref_mut() { progress.set_show_history(show); },
                        journal_movie::Action::MarkTasks(marked,ids) => if let Some(progress) = data.progress.as_deref_mut() { for id in ids { progress.set_task_marked(id,marked); } },
                        journal_movie::Action::Show(_) | journal_movie::Action::ShowMap(_) => {},
                        journal_movie::Action::PlayAudiograph { item, path } => {
                            let entry = inventory.audiographs().iter().find(|entry| entry.item_id == item && entry.path.eq_ignore_ascii_case(&path)).ok_or("audiograph is no longer admitted")?;
                            data.audio.request(item, entry.declaration.clone(), data.loading.catalog.clone())?;
                        },
                        journal_movie::Action::PurchasePower { root, id } => {
                            let powers = data.powers.as_deref().ok_or("Journal power-system owner is absent")?;
                            let success = powers.purchase(inventory, root, id);
                            let runes = inventory.quantity(&powers.definitions.1);
                            self.send(epoch, Operation::PurchaseResult { success, runes }, commands);
                        },
                    }
                }
            },
        }
        Ok(())
    }
}
