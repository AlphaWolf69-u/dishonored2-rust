//! Authored bus ducking (14111efc0/141120b80/1411210f0). Activity is the
//! source bus's first/last playing voice, independently of audio amplitude.
use super::mix::{interpolate, ln, power10, Mixer};
use crate::assets::wwise::BusDuck;
use std::{cmp::Reverse, collections::{BinaryHeap, HashMap}};

const FRAMES: u32 = 1024; // 142427c30
const RATE: u32 = 48000; // 142427c24
const TICK_MS: i32 = 21; // 142427c20; transitions round up in these ticks.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Released, Active, Recovery }

struct Ducker {
    id: u32,
    definitions: Vec<BusDuck>,
    recovery: u32,
    state: State,
}

/// The gain-domain transition configured by 141120a90's 0x101 descriptor.
/// 141130c50 converts dB endpoints to linear gain; 141130ab0 converts each
/// reported value back with the executable's logarithm approximation.
struct Fade {
    start: f32,
    target: f32,
    current: f32,
    processed: bool,
    begin: u32,
    duration: u32,
    curve: u32,
}
impl Fade {
    fn new(start_db: f32, target_db: f32, milliseconds: u32, curve: u32, tick: u32) -> Self {
        let start = power10(start_db * 0.05);
        let target = power10(target_db * 0.05);
        Self { start, target, current: start, processed: false, begin: tick,
            duration: ticks(milliseconds), curve: directed_curve(start, target, curve) }
    }

    /// 1410effb0: relative retargeting multiplies the previous gain target.
    /// Once a value has been reported it is stored in dB, so the new start
    /// must convert that cached value back to linear gain.
    fn retarget(&mut self, target_db: f32, milliseconds: u32, curve: u32, relative: bool, tick: u32) {
        if self.processed { self.start = power10(self.current * 0.05); }
        let target = power10(target_db * 0.05);
        self.target = if relative { target * self.target } else { target };
        self.curve = directed_curve(self.start, self.target, curve);
        self.duration = ticks(milliseconds);
        self.begin = tick;
    }

    fn advance(&mut self, tick: u32) -> (f32, bool) {
        let done = tick >= self.begin.wrapping_add(self.duration);
        let gain = if done { self.target } else {
            let alpha = if self.begin < tick { tick.wrapping_sub(self.begin) as f32 / self.duration as f32 } else { 0.0 };
            // Conversion-table constant segments are handled outside
            // 1410cd3b0; the transition helper's unknown-curve branch is zero.
            if self.curve <= 8 { interpolate(alpha, self.start, self.target, self.curve) } else { 0.0 }
        };
        self.current = ln(gain) * 0.4342945 * 20.0;
        self.processed = true;
        (self.current, done)
    }
}

fn ticks(milliseconds: u32) -> u32 {
    ((milliseconds as i32).wrapping_add(TICK_MS - 1) / TICK_MS) as u32
}
fn directed_curve(start: f32, target: f32, curve: u32) -> u32 {
    if start < target || matches!(curve, 3 | 5) { curve } else { 8u32.wrapping_sub(curve) }
}

struct Entry {
    source: u32,
    target: u32,
    property: u8,
    current: f32,
    fade: Option<Fade>,
}
impl Entry {
    fn change(&mut self, value: f32, milliseconds: u32, curve: u32, relative: bool, tick: u32) {
        if let Some(fade) = &mut self.fade {
            fade.retarget(value, milliseconds, curve, relative, tick);
        } else if self.current != value && milliseconds != 0 {
            self.fade = Some(Fade::new(self.current, value, milliseconds, curve, tick));
        } else { self.current = value; }
    }
}

pub struct Ducking {
    sources: Vec<Ducker>,
    indices: HashMap<u32, usize>,
    entries: Vec<Entry>,
    entry_indices: HashMap<(u32, u32, u8), usize>,
    minimum: HashMap<u32, f32>,
    values: HashMap<u32, [f32; 2]>,
    pending: BinaryHeap<Reverse<(u64, u64, usize)>>,
    order: u64,
}
impl Ducking {
    pub fn synchronize(&mut self, activity: &super::activity::Activity, tick: u64) {
        activity.drain(|id, active| self.set_active(id, active, tick));
    }

    pub fn publish(&self, activity: &super::activity::Activity) {
        for (&id, &value) in &self.values { activity.publish(id, value); }
    }

    pub fn new(mixer: &Mixer) -> Result<Self, String> {
        let mut result = Self { sources: Vec::new(), indices: HashMap::new(), entries: Vec::new(),
            entry_indices: HashMap::new(), minimum: HashMap::new(), values: HashMap::new(),
            pending: BinaryHeap::new(), order: 0 };
        for &id in &mixer.bus_order {
            let bus = mixer.buses.get(&id).ok_or("missing ducking bus")?;
            let settings = bus.bus_settings.as_ref().ok_or("ducking bus has no settings")?;
            result.minimum.insert(id, settings.max_duck_volume);
            result.values.insert(id, [0.0; 2]);
            if settings.ducks.is_empty() { continue; }
            if settings.ducks.iter().any(|duck| !matches!(duck.property, 0 | 5)) {
                return Err(format!("bus {id:#010x} ducks a property outside the native volume lists"));
            }
            // 141120750 converts recovery milliseconds with integer rate/1000
            // and disables holds no longer than one processing quantum.
            let recovery = (RATE / 1000).wrapping_mul(settings.recovery_ms);
            let recovery = if recovery <= FRAMES { 0 } else { recovery.div_ceil(FRAMES) };
            result.indices.insert(id, result.sources.len());
            result.sources.push(Ducker { id, definitions: settings.ducks.clone(), recovery, state: State::Released });
        }
        result.aggregate();
        Ok(result)
    }

    pub fn state(&self, source: u32) -> Option<State> { self.indices.get(&source).map(|&index| self.sources[index].state) }

    /// The bus play-count owner calls this on transitions between zero and
    /// nonzero counts. A paused voice keeps its play-count registration.
    pub fn set_active(&mut self, source: u32, active: bool, tick: u64) {
        let Some(&index) = self.indices.get(&source) else { return };
        if active {
            if self.sources[index].state == State::Active { return; }
            self.change(index, State::Active, tick);
        } else {
            if self.sources[index].state != State::Active { return; }
            let recovery = self.sources[index].recovery;
            if recovery == 0 { self.change(index, State::Released, tick); }
            else {
                self.change(index, State::Recovery, tick);
                // 14111fdc0 creates a delayed action without retaining a
                // cancellation handle. Restarting does not cancel older
                // actions; each callback releases only a current recovery.
                self.pending.push(Reverse((tick + u64::from(recovery), self.order, index)));
                self.order = self.order.wrapping_add(1);
            }
        }
        self.aggregate();
    }

    fn change(&mut self, index: usize, state: State, tick: u64) {
        self.sources[index].state = state;
        let source = self.sources[index].id;
        for definition in &self.sources[index].definitions {
            if !self.minimum.contains_key(&definition.bus) { continue; } // 1411210f0's missing bus lookup.
            if state == State::Recovery {
                // 14111f8a0 freezes both native volume lists for this source.
                for entry in self.entries.iter_mut().filter(|entry| entry.source == source && entry.target == definition.bus) {
                    entry.change(entry.current, 0, 4, false, tick as u32);
                }
                continue;
            }
            let key = (definition.bus, source, definition.property);
            let index = match self.entry_indices.get(&key) {
                Some(&index) => index,
                None if state == State::Active => {
                    let index = self.entries.len();
                    self.entries.push(Entry { source, target: definition.bus, property: definition.property, current: 0.0, fade: None });
                    self.entry_indices.insert(key, index);
                    index
                },
                None => continue,
            };
            let (value, duration) = if state == State::Active { (definition.volume, definition.fade_out_ms) }
                else { (0.0, definition.fade_in_ms) };
            self.entries[index].change(value, duration, u32::from(definition.curve), state == State::Active, tick as u32);
        }
    }

    pub fn advance(&mut self, tick: u64) {
        while self.pending.peek().is_some_and(|Reverse((due, _, _))| *due <= tick) {
            if let Some(Reverse((_, _, index))) = self.pending.pop() {
                if self.sources[index].state == State::Recovery { self.change(index, State::Released, tick); }
            }
        }
        for entry in &mut self.entries {
            if let Some(fade) = &mut entry.fade {
                let (value, done) = fade.advance(tick as u32);
                entry.current = value;
                if done { entry.fade = None; }
            }
        }
        self.aggregate();
    }

    /// 14111ead0 adds source entries in registration order, then applies
    /// the receiving bus's maximum-duck floor. The two properties are
    /// independent: voice volume (0) and bus output volume (5).
    fn aggregate(&mut self) {
        for value in self.values.values_mut() { *value = [0.0; 2]; }
        for entry in &self.entries {
            if let Some(value) = self.values.get_mut(&entry.target) { value[usize::from(entry.property == 5)] += entry.current; }
        }
        for (id, value) in &mut self.values {
            if let Some(&minimum) = self.minimum.get(id) {
                for value in value { if *value < minimum { *value = minimum; } }
            }
        }
    }

    pub fn value(&self, bus: u32, property: u8) -> Result<f32, String> {
        if !matches!(property, 0 | 5) { return Err("ducking volume property is neither voice nor bus output".into()); }
        Ok(self.values.get(&bus).map_or(0.0, |value| value[usize::from(property == 5)]))
    }
}
