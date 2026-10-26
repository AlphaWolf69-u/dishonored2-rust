//! 79e600/79e940/79e2f0 world-FX node lifetime and its two effect observers.
//! The graph host resolves connected inputs, entities and their actual clocks.

pub struct CreatedEffect {
    pub entity: u32,
    pub clock: f64,
}

/// Hooks may deliver the effect observers synchronously. Passing the live node
/// preserves the source's stop/callback/output/reset ordering under reentry.
pub trait FxNodeWorld {
    fn duration_seconds(&mut self) -> Result<f32, String>;
    fn spawn(&mut self, node: &mut FxNode, duration: f32) -> Result<Option<CreatedEffect>, String>;
    fn stop_effect(&mut self, node: &mut FxNode, handle: u32) -> Result<(), String>;
    fn emit_completion(&mut self, node: &mut FxNode) -> Result<(), String>;
    fn entity_clock(&mut self, entity: u32) -> Result<Option<f64>, String>;
}

pub struct FxNode {
    effect: Option<u32>,
    clock_entity: Option<u32>,
    deadline: f64,
    stopped: bool,
}
impl Default for FxNode {
    fn default() -> Self {
        Self {
            effect: None,
            clock_entity: None,
            deadline: -1.,
            stopped: false,
        }
    }
}
#[derive(Clone, Copy)]
pub struct FxNodeSavedState {
    pub active: bool,
    pub timed: bool,
    pub remaining: f32,
}
impl FxNode {
    pub fn scheduled(&self) -> bool {
        self.clock_entity.is_some()
    }
    fn clear(&mut self) {
        self.effect = None;
        self.clock_entity = None;
        self.deadline = -1.;
        self.stopped = false;
    }
    /// 79e2c0: creation observer receives the manager's actual instance ID.
    pub fn effect_started(&mut self, handle: u32) {
        self.effect = (handle != u32::MAX).then_some(handle);
    }
    /// 79e250 ignores completion from an earlier, replaced effect instance.
    pub fn effect_completed(
        &mut self,
        handle: u32,
        world: &mut impl FxNodeWorld,
    ) -> Result<(), String> {
        if self.effect.unwrap_or(u32::MAX) == handle {
            if self.effect.is_some() {
                world.emit_completion(self)?;
            }
            self.clear();
        }
        Ok(())
    }
    /// The native Start input stops the previous instance before emitting its
    /// completion and clearing state; spawning zero duration remains a no-op.
    pub fn start(&mut self, world: &mut impl FxNodeWorld) -> Result<(), String> {
        if let Some(handle) = self.effect {
            world.stop_effect(self, handle)?;
        }
        if self.effect.is_some() {
            world.emit_completion(self)?;
        }
        self.clear();
        let duration = world.duration_seconds()?;
        self.spawn(world, duration)
    }
    fn spawn(&mut self, world: &mut impl FxNodeWorld, duration: f32) -> Result<(), String> {
        // Start reaches79e6a0 even for zero: it resolves the connected FX and
        // location before rejecting zero duration. The host owns that lookup.
        if let Some(created) = world.spawn(self, duration)? {
            if 0. < duration {
                self.clock_entity = Some(created.entity);
                self.deadline = f64::from(duration) + created.clock;
            }
        }
        Ok(())
    }
    pub fn stop(&mut self, world: &mut impl FxNodeWorld) -> Result<(), String> {
        self.stopped = true;
        if let Some(handle) = self.effect {
            world.stop_effect(self, handle)?;
        }
        Ok(())
    }
    /// 79e2f0's strict pre-deadline predicate; equality finishes the task.
    pub fn update(&mut self, world: &mut impl FxNodeWorld) -> Result<bool, String> {
        if let Some(entity) = self.clock_entity {
            if let Some(now) = world.entity_clock(entity)? {
                if now <= self.deadline && self.deadline != now {
                    return Ok(true);
                }
            }
        }
        if let Some(handle) = self.effect {
            world.stop_effect(self, handle)?;
        }
        Ok(false)
    }
    /// 79e480 stores remaining source-clock seconds, not a wall-clock deadline.
    pub fn save(&self, world: &mut impl FxNodeWorld) -> Result<FxNodeSavedState, String> {
        let clock = if !(self.deadline < 0.) {
            match self.clock_entity {
                Some(entity) => world.entity_clock(entity)?,
                None => None,
            }
        } else {
            None
        };
        Ok(FxNodeSavedState {
            active: self.effect.is_some(),
            timed: clock.is_some(),
            remaining: clock.map_or_else(
                || {
                    if self.stopped {
                        f32::from_bits(0x7149_f2ca)
                    } else {
                        0.
                    }
                },
                |now| (self.deadline - now) as f32,
            ),
        })
    }
    /// 79e3a0 consumes the saved state once. Untimed zero remaining duration
    /// re-reads the current connected duration input, as the game does.
    pub fn restore(
        &mut self,
        saved: FxNodeSavedState,
        world: &mut impl FxNodeWorld,
    ) -> Result<(), String> {
        if saved.active {
            if saved.timed && saved.remaining != 0. {
                self.spawn(world, saved.remaining)?;
            } else if !saved.timed && saved.remaining == 0. {
                let duration = world.duration_seconds()?;
                if duration != 0. {
                    self.spawn(world, duration)?;
                }
            } else if !saved.timed {
                self.stopped = true;
            }
            if self.stopped {
                world.emit_completion(self)?;
                self.clear();
            }
        }
        Ok(())
    }
}
