//! Bus play-count registrations (1410e02a0/1410dd980/14111efc0).
//! Sound lifetimes register every output-bus ancestor, including logical
//! buses. Pausing preserves registration; destroying the sound releases it.
use super::mix::Mixer;
use std::{collections::HashMap, sync::{Arc, Mutex, atomic::{AtomicU32, Ordering}}};

struct Bus {
    id: u32,
    ducker: bool,
    parent: Option<usize>,
    mixing: bool,
    count: AtomicU32,
    ducking: [AtomicU32; 2],
}
pub struct Activity {
    buses: Vec<Bus>,
    indices: HashMap<u32, usize>,
    edges: Mutex<Vec<(u32, bool)>>,
}
#[derive(Clone)]
pub struct Registration { owner: Arc<Activity>, first: Option<usize> }
pub struct Token { registration: Registration }

impl Activity {
    pub fn new(mixer: &Mixer) -> Result<Self, String> {
        let indices = mixer.bus_order.iter().enumerate().map(|(index, &id)| (id, index)).collect::<HashMap<_, _>>();
        let mut buses = Vec::new();
        for &id in &mixer.bus_order {
            mixer.mixing_bus(id)?; // Validate the complete parent chain once.
            let bus = mixer.buses.get(&id).ok_or("missing activity bus")?;
            let parent = if bus.parent == 0 { None } else { Some(*indices.get(&bus.parent).ok_or("missing activity bus parent")?) };
            buses.push(Bus { id, ducker: bus.bus_settings.as_ref().is_some_and(|settings| !settings.ducks.is_empty()),
                parent, mixing: mixer.bus_mixes(id)?, count: AtomicU32::new(0),
                ducking: [AtomicU32::new(0), AtomicU32::new(0)] });
        }
        Ok(Self { buses, indices, edges: Mutex::new(Vec::new()) })
    }

    pub fn registration(self: &Arc<Self>, id: u32) -> Result<Registration, String> {
        let first = if id == 0 { None } else { Some(*self.indices.get(&id).ok_or_else(|| format!("unknown activity bus {id:#010x}"))?) };
        Ok(Registration { owner: self.clone(), first })
    }

    /// Preserve first/last play-count edges even when both occur inside one
    /// output quantum. Polling the final count loses short container voices.
    pub(super) fn drain(&self, mut visit: impl FnMut(u32, bool)) {
        let mut edges = self.edges.lock().unwrap_or_else(|error| error.into_inner());
        for (id, active) in edges.drain(..) { visit(id, active); }
    }

    pub(super) fn publish(&self, id: u32, values: [f32; 2]) {
        if let Some(&index) = self.indices.get(&id) {
            for (target, value) in self.buses[index].ducking.iter().zip(values) { target.store(value.to_bits(), Ordering::Relaxed); }
        }
    }

    pub fn own_ducking(&self, id: u32, property: u8) -> Result<f32, String> {
        if !matches!(property, 0 | 5) { return Err("activity ducking property is neither voice nor bus output".into()); }
        Ok(self.indices.get(&id).map_or(0.0, |&index| f32::from_bits(self.buses[index].ducking[usize::from(property == 5)].load(Ordering::Relaxed))))
    }

    /// Voice-volume reductions propagate through all bus ancestors.
    pub fn voice_ducking(&self, id: u32) -> f32 { self.fold(id, false, false) }

    /// Output gain folds logical ancestors, stopping before a mixing parent
    /// (14111e660). An incoming direct connection skips a mixing first bus.
    pub fn output_ducking(&self, id: u32, direct: bool) -> f32 { self.fold(id, true, direct) }

    fn fold(&self, id: u32, output: bool, direct: bool) -> f32 {
        let mut current = self.indices.get(&id).copied();
        let mut first = true;
        let mut value = 0.0;
        while let Some(index) = current {
            let bus = &self.buses[index];
            if output && bus.mixing && (!first || direct) { break; }
            value += f32::from_bits(bus.ducking[usize::from(output)].load(Ordering::Relaxed));
            first = false;
            current = bus.parent;
        }
        value
    }
}

impl Registration {
    pub fn start(&self) -> Token {
        let mut edges = self.owner.edges.lock().unwrap_or_else(|error| error.into_inner());
        let mut current = self.first;
        while let Some(index) = current {
            let bus = &self.owner.buses[index];
            let previous = bus.count.fetch_add(1, Ordering::Relaxed) as u16;
            if bus.ducker && previous == 0 { edges.push((bus.id, true)); }
            current = bus.parent;
        }
        Token { registration: self.clone() }
    }
}
impl Drop for Token {
    fn drop(&mut self) {
        let mut edges = self.registration.owner.edges.lock().unwrap_or_else(|error| error.into_inner());
        let mut current = self.registration.first;
        while let Some(index) = current {
            let bus = &self.registration.owner.buses[index];
            let previous = bus.count.fetch_sub(1, Ordering::Relaxed) as u16;
            if bus.ducker && previous == 1 { edges.push((bus.id, false)); }
            current = bus.parent;
        }
    }
}
