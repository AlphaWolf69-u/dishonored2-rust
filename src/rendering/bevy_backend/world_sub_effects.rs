//! The FX group's m_subFXs family (arkFXSubFXDesc): nested FX played on a
//! repeating schedule, each pick a whole effect whose groups start in the
//! sub-FX's own chains and share the owner's clock and random stream.
//! Start 2307f0, pick 2306e0, trigger loop 2305c0, update 230980,
//! stop 230960, child reset 223dd0 / update 223f60, teardown 2307e0.
use super::{particle_systems::ParticleSystems, world_sounds::WorldSounds};
use crate::rendering::particles::world_sources::WorldEffectDefinition;
use std::sync::Arc;

type SoundSystems = crate::gameplay::effects::sound::systems::SoundSystems<
    crate::gameplay::sound::Emitter,
    crate::gameplay::sound::playing::PlayingId,
>;

/// The chains a stop or release touches.
pub(super) struct Chains<'a> {
    pub(super) systems: &'a mut ParticleSystems,
    pub(super) decals: &'a mut super::decal_effects::DecalSystems,
    pub(super) sound_systems: &'a mut SoundSystems,
    pub(super) sounds: &'a WorldSounds,
}

/// 2305c0 keeps triggering while the period is at least this long (141ab3b04).
const MINIMUM_PERIOD: f32 = 0.001;

pub(super) struct SubEffect {
    owner: Arc<WorldEffectDefinition>,
    group: usize,
    index: usize,
    /// arkStandardEventTimings state (+0x3c): next trigger, remaining cycles
    /// (0: endless), current period.
    next: f32,
    remaining: u32,
    period: f32,
    /// Sequential mode's current declaration (+0x48).
    sequence: usize,
    /// +0x4c active, +0x4d active or children alive, +0x4e stop issued.
    active: bool,
    live: bool,
    stopped: bool,
    pub(super) head: u16,
    pub(super) sound_head: u16,
    pub(super) decal_head: u16,
    pub(super) children: Vec<SubEffect>,
}

impl SubEffect {
    fn asset(&self) -> &crate::assets::fx::SubEffectAsset {
        &self.owner.groups[self.group].asset.sub_effects[self.index]
    }

    /// 222f80's admission: an enabled descriptor with any non-null declaration (230590).
    pub(super) fn admitted(asset: &crate::assets::fx::SubEffectAsset) -> bool {
        asset.enabled && asset.effects.iter().any(Option::is_some)
    }

    /// 2307f0 with the owning group's sampled offsets.
    pub(super) fn start(
        owner: Arc<WorldEffectDefinition>,
        group: usize,
        index: usize,
        offsets: [f32; 4],
        seed: &mut u32,
    ) -> Self {
        let asset = &owner.groups[group].asset.sub_effects[index];
        // 216c00: cycles (when set), then the delay, then (216b80) the period.
        let remaining = if asset.cycles == [0, 0] {
            0
        } else {
            let unit = crate::random::lcg_unit(seed);
            let [maximum, minimum] = asset.cycles.map(|v| v as f32);
            let count = (unit * (maximum - minimum) + minimum) as i32;
            count.max(1) as u32
        };
        let delay = crate::gameplay::effects::parameters::sample_range(
            [asset.delay[1], asset.delay[0]],
            seed,
        );
        let period = crate::gameplay::effects::parameters::sample_range(
            [asset.period[1], asset.period[0]],
            seed,
        );
        let count = asset.effects.len();
        let mut sequence = 0;
        if asset.sequential {
            if asset.random_start && count != 0 {
                sequence = ((crate::random::lcg_step(seed) >> 10) & 0x7fff) as usize % count;
            }
            if sequence >= count {
                sequence = 0;
            }
            while asset.effects[sequence].is_none() {
                sequence = (sequence + 1) % count;
            }
        }
        Self {
            next: crate::float::add(delay, offsets[0]),
            remaining,
            period,
            sequence,
            active: true,
            live: true,
            stopped: false,
            head: u16::MAX,
            sound_head: u16::MAX,
            decal_head: u16::MAX,
            children: Vec::new(),
            owner,
            group,
            index,
        }
    }

    /// 2306e0: the declaration of this trigger.
    fn pick(&mut self, seed: &mut u32) -> Option<String> {
        let asset = &self.owner.groups[self.group].asset.sub_effects[self.index];
        let effects = &asset.effects;
        let count = effects.len();
        if !asset.sequential {
            if count == 0 {
                return None;
            }
            let mut index = ((crate::random::lcg_step(seed) >> 10) & 0x7fff) as usize % count;
            let mut tries = count - 1;
            while effects[index].is_none() && tries != 0 {
                index = (index + 1) % count;
                tries -= 1;
            }
            return effects[index].clone();
        }
        let current = self.sequence;
        let mut next = (current + 1) % count;
        while effects[next].is_none() {
            next = (next + 1) % count;
        }
        self.sequence = next;
        effects[current].clone()
    }

    /// 230960: deactivate; the first stop resets the child chains at `now`.
    pub(super) fn stop(&mut self, now: f32, context: &mut Chains) -> Result<(), String> {
        self.active = false;
        if !self.stopped {
            self.stopped = true;
            self.reset(now, context)?;
        }
        Ok(())
    }

    /// 223dd0: every family chain of the child stops at `now`.
    fn reset(&mut self, now: f32, context: &mut Chains) -> Result<(), String> {
        context.systems.stop(self.head, now);
        context.decals.stop(self.decal_head, now);
        context
            .sound_systems
            .stop(self.sound_head, now, &mut context.systems.seed)?;
        for child in &mut self.children {
            child.stop(now, context)?;
        }
        Ok(())
    }

    /// 2307e0 / 222e10: release the child chains.
    pub(super) fn clear(&mut self, context: &mut Chains) -> Result<(), String> {
        context.systems.clear(&mut self.head)?;
        context.decals.clear(&mut self.decal_head, |_| {})?;
        if self.sound_head != u16::MAX {
            let mut sounds = context.sounds.clone();
            context
                .sound_systems
                .clear(&mut self.sound_head, &mut sounds)?;
        }
        for child in &mut self.children {
            child.clear(context)?;
        }
        self.children.clear();
        Ok(())
    }

    /// 230980 after 223f60 has updated the child chains (`children_alive`).
    /// Returns +0x4d: whether this sub-FX stays linked.
    pub(super) fn advance(
        &mut self,
        now: f32,
        delta: f32,
        children_alive: bool,
        context: &mut super::world_particle_state::GroupContext,
    ) -> Result<bool, String> {
        self.live = children_alive;
        if self.active {
            if children_alive && self.asset().wait_end {
                // 216b70: postpone the next trigger while the child plays.
                self.next = crate::float::add(self.next, delta);
            } else {
                self.active = self.trigger(now, context)?;
            }
        }
        self.live |= self.active;
        Ok(self.live)
    }

    /// 2305c0: every trigger due before `now`; false ends the schedule.
    fn trigger(
        &mut self,
        now: f32,
        context: &mut super::world_particle_state::GroupContext,
    ) -> Result<bool, String> {
        while self.next < now {
            self.live = true;
            let picked = self.pick(&mut context.systems.seed);
            self.reset(now, &mut context.chains())?;
            if let Some(name) = picked {
                let definition = context
                    .effects
                    .get(&name)
                    .ok_or_else(|| format!("sub-FX {name} was not prepared"))?
                    .as_ref()
                    .map_err(Clone::clone)?
                    .clone();
                super::world_particle_state::start_groups(
                    &definition,
                    now,
                    &mut self.head,
                    &mut self.sound_head,
                    &mut self.decal_head,
                    &mut self.children,
                    context,
                )?;
            }
            if self.remaining != 0 {
                self.remaining -= 1;
                if self.remaining == 0 {
                    return Ok(false);
                }
            }
            self.next = crate::float::add(self.next, self.period);
            let asset = self.asset();
            let range = [asset.period[1], asset.period[0]];
            self.period = crate::gameplay::effects::parameters::sample_range(
                range,
                &mut context.systems.seed,
            );
            if self.period < MINIMUM_PERIOD {
                break;
            }
        }
        Ok(true)
    }
}
