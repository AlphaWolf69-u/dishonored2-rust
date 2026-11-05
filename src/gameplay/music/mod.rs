//! Level music and ambiance manager (1406637e0, 1406647d0, 1406641e0).
//! Uses the worldspawn's existing audio component and the shared entity-event
//! dispatcher. It owns no sound banks, source bytes, decoder or voice cache.
mod declaration;
mod requests;
pub use declaration::{Definition, Event};
pub use requests::Notification;
use requests::{Requests, INVALID_REQUEST};
use std::sync::Arc;

/// Adapter to the existing audio/event owners. Its lifetime is the admitted
/// worldspawn generation, with the live player/sensor dependencies verified.
pub trait Host<E> {
    fn post(&mut self, event: &Event) -> bool;
    /// Request event 15 targets the actual worldspawn, not the player.
    fn request_receiver(&mut self, enabled: bool);
    /// Player events 10e/10f/110/111/144/145/4e, in that registration order.
    fn player_receivers(&mut self, enabled: bool);
    /// Event 16 with [handle, operation]; suppress if its owner is retired.
    fn notify(&mut self, notification: Notification);
    /// idDeclEntityDef ancestry (1400147a0), not a class/name heuristic.
    fn entity_inherits(&self, entity: E, declaration: &str) -> Result<bool, String>;
}

pub struct Manager<E> {
    definition: Arc<Definition>,
    requests: Requests,
    stage_handles: [i32; 8],
    tracked: Vec<E>,
    combatants: Vec<E>,
    ambiance_started: bool,
    current_ambiance: Option<Arc<Event>>,
    next_ambiance: Option<Arc<Event>>,
    last_stinger: [f64; 3],
}

impl<E: Copy + Eq> Manager<E> {
    /// Called after the original game-feature 0x20 gate and actual dependency
    /// admission. Null start/stop music declarations leave the manager absent.
    pub fn start(definition: Arc<Definition>, host: &mut impl Host<E>) -> Option<Self> {
        let start = definition.start_music.as_ref()?;
        definition.stop_music.as_ref()?;
        host.request_receiver(true); // 140663af0 also resets list and handles.
        host.post(start);
        let mut value = Self { definition, requests: Requests::default(),
            stage_handles: [INVALID_REQUEST; 8], tracked: Vec::new(), combatants: Vec::new(),
            ambiance_started: false, current_ambiance: None, next_ambiance: None,
            last_stinger: [-1.0; 3] };
        value.enable_stage(0);
        if let Some(event) = &value.definition.start_ambiance {
            value.ambiance_started = host.post(event);
        }
        host.player_receivers(true);
        Some(value)
    }

    /// 1406641e0 posts stopAmbiance, resets the request owner and unsubscribes.
    /// It does not post stopMusic; the audio component owns voice teardown.
    /// Consuming this level owner releases all declaration/identity references.
    pub fn stop(mut self, host: &mut impl Host<E>) {
        if let Some(event) = &self.definition.stop_ambiance { host.post(event); }
        self.ambiance_started = false;
        host.request_receiver(false);
        // 14054ced0 clears entries, expiration count and both request IDs.
        self.requests = Requests::default();
        self.stage_handles.fill(INVALID_REQUEST);
        host.player_receivers(false);
    }

    fn enable_stage(&mut self, stage: usize) {
        if self.stage_handles[stage] < 0 {
            self.stage_handles[stage] = self.requests.insert(stage as i32, 0.0, 0, false, false);
        }
    }
    fn disable_stage(&mut self, stage: usize, host: &mut impl Host<E>) {
        if self.stage_handles[stage] >= 0 {
            self.requests.remove(self.stage_handles[stage], &mut |notification| host.notify(notification));
            self.stage_handles[stage] = INVALID_REQUEST;
        }
    }

    /// 1406647d0: sensor subtype 0x1d's virtual+d0 supplies suspense; worldspawn
    /// timespace supplies dt. Only a changed winning handle posts a stage.
    pub fn update(&mut self, dt: f32, suspense: bool, host: &mut impl Host<E>) {
        if suspense { self.enable_stage(1); } else { self.disable_stage(1, host); }
        if self.requests.update(dt, &mut |notification| host.notify(notification)) {
            if let Some(event) = self.requests.stage().and_then(|stage|
                usize::try_from(stage).ok().and_then(|index| self.definition.stages.get(index)))
                .and_then(Option::as_ref) {
                host.post(event);
            }
        }
        if self.ambiance_started && !same_event(&self.current_ambiance, &self.next_ambiance) {
            if let Some(event) = &self.next_ambiance { host.post(event); }
            self.current_ambiance = self.next_ambiance.clone();
        }
    }

    /// The original setter 1406640a0 retains the selected declaration pointer.
    pub fn set_ambiance(&mut self, event: Option<Arc<Event>>) { self.next_ambiance = event; }

    pub fn add_request(&mut self, stage: i32, duration: f32, lifetime: i32, notify: bool) -> i32 {
        self.requests.insert(stage, duration, lifetime, notify, false)
    }
    pub fn remove_request(&mut self, handle: i32, host: &mut impl Host<E>) {
        self.requests.remove(handle, &mut |notification| host.notify(notification));
    }

    /// Raw registered entity events retain the exact two source sets. 10e adds
    /// only to +168; it does not reclassify stages until the original does.
    pub fn entity_event(&mut self, event: u16, entity: E, host: &mut impl Host<E>) -> Result<(), String> {
        match event {
            0x10e => { if !self.tracked.contains(&entity) { self.tracked.push(entity); } },
            0x10f => {
                remove(&mut self.tracked, entity);
                if self.tracked.is_empty() { self.disable_stage(2, host); }
            },
            0x110 => {
                if !self.combatants.contains(&entity) { self.combatants.push(entity); }
                self.disable_stage(2, host);
                self.classify(host)?;
            },
            0x111 => { remove(&mut self.combatants, entity); self.classify(host)?; },
            _ => return Err(format!("unsupported music entity event {event:#x}")),
        }
        Ok(())
    }
    /// Registered events 144 and 145 use the persistent stress-stage handle.
    pub fn stress(&mut self, enabled: bool, host: &mut impl Host<E>) {
        if enabled { self.enable_stage(3); } else { self.disable_stage(3, host); }
    }

    /// 1406649b0 evaluates elite ancestry first, special ancestry second.
    fn classify(&mut self, host: &mut impl Host<E>) -> Result<(), String> {
        let mut ordinary = self.combatants.len() as i32;
        let mut elite = 0;
        let mut special = 0;
        for &entity in &self.combatants {
            let mut matched = false;
            for declaration in &self.definition.elite_enemies {
                if host.entity_inherits(entity, declaration)? { elite += 1; matched = true; break; }
            }
            if !matched {
                for declaration in &self.definition.special_enemies {
                    if host.entity_inherits(entity, declaration)? { special += 1; matched = true; break; }
                }
            }
            if matched { ordinary -= 1; }
        }
        if special > 0 { self.enable_stage(7); } else { self.disable_stage(7, host); }
        let [guard_threshold, elite_threshold] = self.definition.combat_level_3;
        if ordinary >= guard_threshold || elite >= elite_threshold { self.enable_stage(6); }
        else { self.disable_stage(6, host); }
        let [guard_threshold, elite_threshold] = self.definition.combat_level_2;
        if ordinary >= guard_threshold || elite >= elite_threshold { self.enable_stage(5); }
        else { self.disable_stage(5, host); }
        if self.combatants.is_empty() {
            self.disable_stage(4, host);
            if !self.tracked.is_empty() { self.enable_stage(2); }
        } else { self.enable_stage(4); }
        Ok(())
    }

    /// 1406634c0 uses monotonic QPC seconds, not game/paused time. The double
    /// difference is narrowed before a strict cooldown comparison. Level zero
    /// is admitted only for stages 0/1/3 (source byte mask 0x01000101), or empty.
    pub fn stinger(&mut self, level: usize, real_seconds: f64, host: &mut impl Host<E>) -> Result<(), String> {
        let stinger = self.definition.stingers.get(level).ok_or("invalid music stinger level")?;
        let elapsed = (real_seconds - self.last_stinger[level]) as f32;
        if elapsed > stinger.cooldown {
            if let Some(event) = &stinger.event {
                if level != 0 || self.requests.stage().is_none_or(|stage| matches!(stage, 0 | 1 | 3)) {
                    host.post(event);
                    self.last_stinger[level] = real_seconds;
                }
            }
        }
        Ok(())
    }
}

fn same_event(a: &Option<Arc<Event>>, b: &Option<Arc<Event>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.declaration == b.declaration,
        _ => false,
    }
}
fn remove<E: Eq>(entries: &mut Vec<E>, entity: E) {
    if let Some(index) = entries.iter().position(|entry| *entry == entity) { entries.remove(index); }
}
