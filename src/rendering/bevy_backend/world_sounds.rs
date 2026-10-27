//! The sound family shares allocation order/RNG with render-owned FX, while
//! actual event posts and completion callbacks run with the main audio owner.
use bevy::{prelude::*, render::extract_resource::ExtractResource};
use std::{collections::HashMap, sync::{Arc, Mutex, MutexGuard}};
use crate::gameplay::{effects::sound::{systems::SoundSystems, SoundEvent}, sound::Emitter};

#[derive(Clone, Copy)]
pub(crate) struct SoundFrame {
    pub age: f32,
    pub origin: Vec3,
    pub axis: Mat3,
    pub scale: f32,
}

pub(crate) struct SoundGroup {
    pub head: u16,
    pub frame: Option<SoundFrame>,
    pub clear: bool,
}

#[derive(Default)]
pub(crate) struct SoundState {
    pub systems: SoundSystems<Emitter, Entity>,
    pub groups: HashMap<u32, SoundGroup>,
    /// Readiness is published only after the installed event has decoded.
    pub events: HashMap<u32, Result<(), String>>,
}

#[derive(Resource, Clone, Default, ExtractResource)]
pub(crate) struct WorldSounds(Arc<Mutex<SoundState>>);

impl WorldSounds {
    pub fn lock(&self) -> MutexGuard<'_, SoundState> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn start(&self, parent: u32, position: [Vec3; 2], start: SoundEvent,
        stop: SoundEvent, age: f32, allow_virtualization: bool, seed: &mut u32) -> Result<(), String>
    {
        let mut state = self.lock();
        let SoundState { systems, groups, .. } = &mut *state;
        let group = groups.entry(parent).or_insert(SoundGroup {
            head: u16::MAX, frame: None, clear: false,
        });
        systems.start(&mut group.head, position, start, stop, age, allow_virtualization, seed)
    }

    pub fn update(&self, parent: u32, frame: SoundFrame) -> bool {
        let mut state = self.lock();
        let Some(group) = state.groups.get_mut(&parent) else { return false };
        group.frame = Some(frame);
        group.head != u16::MAX
    }

    pub fn stop(&self, parent: u32, age: f32, seed: &mut u32) -> Result<(), String> {
        let mut state = self.lock();
        if let Some(group) = state.groups.get(&parent) {
            let head = group.head;
            state.systems.stop(head, age, seed)?;
        }
        Ok(())
    }

    pub fn retire(&self, parent: u32) {
        if let Some(group) = self.lock().groups.get_mut(&parent) { group.clear = true; }
    }
}
