//! The pause screen FX (notes-pause-contract.md section 3): the pause state's
//! `m_screenFX` (`models/effects/gui/pause.fx`) is one environment override
//! (`env/fx/ui_pause`: the pause colour cube and depth of field). OnEnter
//! starts it (`m_playFxOnActivation`, default 1), as does the movie's
//! `EnableUIPostFX`; `FUN_1407913f0` advances it on every paused frame by clock
//! group 0's delta (`gameLocal+0x2e7428`, field +0x28), which keeps the last
//! game frame's step while the paused frame skips the clock step; OnExit and
//! `DisableUIPostFX` remove it. Its blend is the weight of its environment
//! in the environment manager's override slots (`display_scene`).
use crate::assets::archive::Catalog;
use crate::gameplay::effects::{
    environment::{EnvironmentBlendEffect, EnvironmentEffectDefinition},
    groups::sample_offsets,
    timing::EffectTimingRanges,
};
use crate::gameplay::game_clock_runtime::{ClockRuntime, GamePaused};
use crate::gameplay::options::decl::{atom, read_edit};
use bevy::prelude::*;
use std::sync::Arc;

const PAUSE_STATE: &str = "ui/gamelogic/state_pause.gamelogicstatepause";

/// The FX record's time scale (`+0xf8`). An entity's render entity copies its
/// clock group's rate (`FUN_1408780a0`, 1.0); the pause FX clears that copy
/// (`FUN_140791240`, flags `+0x138 & 0xfc`) and keeps the record's initial
/// value: `FUN_14019d100` -> `FUN_1401679d0` -> `FUN_14021dc10(rec + 0x38)`
/// stores 1.0 at `+0xc0`.
const TIME_SCALE: f32 = 1.0;

pub(super) struct ScreenFx {
    prepared: Arc<crate::rendering::environment_resources::PreparedEnvironment>,
    _source: Arc<crate::assets::fx::definition::Definition>,
    timings: EffectTimingRanges,
    effect: Arc<EnvironmentEffectDefinition>,
    /// The override environment (`env/fx/ui_pause`).
    pub environment: String,
}

/// The pause state's screen FX: its first enabled group's first enabled
/// environment override.
pub(super) fn load(catalog: &Catalog) -> Result<ScreenFx, String> {
    let mut read = |p: &str| catalog.read(p).ok().flatten();
    let state = read_edit(&mut read, "gamelogicstatepause", PAUSE_STATE)?;
    let name = atom(&state, "m_screenFX")
        .ok_or("the pause state has no m_screenFX")?
        .to_string();
    let source = crate::assets::fx::declaration::acquire(
        catalog,
        &name,
        crate::assets::fx::definition::Consumer::Common,
        None,
    )?;
    if !source.groups_present {
        return Err(format!("{name} has no m_groups"));
    }
    for group in &source.groups {
        let group = &group.asset;
        if !group.enabled {
            continue;
        }
        if let Some(env) = group
            .environments
            .iter()
            .find(|e| e.enabled && e.environment.is_some())
        {
            let prepared = crate::rendering::environment_resources::PreparedEnvironment::acquire(
                catalog,
                env.environment.as_ref().unwrap(),
                0,
            )?;
            return Ok(ScreenFx {
                prepared,
                timings: group.timings,
                effect: env.blend.clone(),
                environment: env.environment.as_ref().unwrap().clone(),
                _source: source.clone(),
            });
        }
    }
    Err(format!("{name} has no environment override"))
}

/// An environment decl's render parameters, following its inherit chain.
pub(super) fn environment_parameters(
    catalog: &Catalog,
    name: &str,
) -> Result<crate::assets::entities::EnvironmentRenderParameters, String> {
    Ok(
        crate::rendering::environment_resources::PreparedEnvironment::acquire(catalog, name, 0)?
            .parameters
            .clone(),
    )
}

struct Active {
    effect: EnvironmentBlendEffect,
    time: f32,
    binding: crate::rendering::environment_effects::EnvironmentEffectBinding<
        Arc<crate::rendering::environment_resources::PreparedEnvironment>,
        u64,
    >,
    lease: crate::rendering::environment_effect_runtime::EnvironmentPoolLease,
}

#[derive(Resource)]
pub(super) struct PauseScreenFx {
    pool: Option<crate::rendering::environment_effect_runtime::SharedEnvironmentPool>,
    retirements: Vec<(u64, Option<usize>)>,
    closed: bool,
    error: Option<String>,
    fx: Option<ScreenFx>,
    active: Option<Active>,
    /// `EnableUIPostFX` (true) / `DisableUIPostFX` (false) from the movie.
    pub requested: Option<bool>,
    was_paused: bool,
    /// The FX family's shared random state (the pause FX's ranges are fixed).
    seed: u32,
    /// The current blend: the override environment's weight.
    pub weight: f32,
}

impl PauseScreenFx {
    pub(super) fn new(fx: Option<ScreenFx>) -> Self {
        Self {
            fx,
            active: None,
            requested: None,
            was_paused: false,
            seed: 0,
            weight: 0.,
            pool: None,
            retirements: Vec::new(),
            closed: false,
            error: None,
        }
    }

    pub(super) fn environment(&self) -> Option<&str> {
        self.fx.as_ref().map(|f| f.environment.as_str())
    }

    /// `FUN_1407912e0` / `FUN_140791240`: a new FX instance at time zero.
    fn start(&mut self) -> Result<bool, String> {
        let Some(fx) = &self.fx else { return Ok(true) };
        if !fx
            .prepared
            .prepared
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(false);
        }
        let Some(pool) = self.pool.clone() else {
            return Ok(false);
        };
        self.remove()?;
        let fx = self.fx.as_ref().unwrap();
        let offsets = sample_offsets(&fx.timings, 0., &mut self.seed);
        let Some(lease) = pool.acquire_external()? else {
            return Ok(true);
        };
        let effect = EnvironmentBlendEffect::new(fx.effect.clone(), offsets, 1, &mut self.seed)?;
        self.active = Some(Active {
            effect,
            time: 0.,
            lease,
            binding: crate::rendering::environment_effects::EnvironmentEffectBinding::new(
                fx.prepared.clone(),
            ),
        });
        self.weight = 0.;
        Ok(true)
    }

    /// `FUN_140791370`: the FX is stopped and removed from its entity.
    fn remove(&mut self) -> Result<(), String> {
        if let Some(active) = &mut self.active {
            active.lease.release()?;
        }
        if let Some(active) = self.active.take() {
            if let Some((view, slot)) = active.binding.into_registration() {
                self.retirements.push((view, Some(slot)));
            }
        }
        self.weight = 0.;
        Ok(())
    }
    pub(super) fn set_environment_pool(
        &mut self,
        pool: crate::rendering::environment_effect_runtime::SharedEnvironmentPool,
    ) -> Result<(), String> {
        if self.pool.as_ref().is_some_and(|old| old.is_same(&pool)) {
            return Ok(());
        }
        if self.active.is_some() {
            return Err("pause environment pool changed while active".into());
        }
        self.pool = Some(pool);
        Ok(())
    }
    pub(super) fn initialize_environment_pool(
        &mut self,
        pool: crate::rendering::environment_effect_runtime::SharedEnvironmentPool,
    ) -> Result<(), String> {
        if self.active.is_some() || !self.retirements.is_empty() {
            return Err("previous pause environment generation is still owned".into());
        }
        self.set_environment_pool(pool)?;
        self.closed = false;
        self.error = None;
        Ok(())
    }
    pub(super) fn environment_error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub(super) fn visit_environment_sources(
        &self,
        mut visit: impl FnMut(&Arc<crate::rendering::environment_resources::PreparedEnvironment>),
    ) {
        if let Some(fx) = &self.fx {
            visit(&fx.prepared);
        }
    }
    pub(super) fn visit_environment_orders(&self, mask: u8, mut visit: impl FnMut(u64)) {
        if let Some(active) = &self.active {
            if active.effect.view_mask & mask != 0 {
                visit(active.effect.registration_order);
            }
        }
    }
    pub(super) fn publish_environment(
        &mut self,
        order: u64,
        view: u64,
        mask: u8,
        slots: &mut crate::rendering::environment_overrides::EnvironmentOverrides<
            Arc<crate::rendering::environment_resources::PreparedEnvironment>,
        >,
    ) -> Result<(), String> {
        let active = self
            .active
            .as_mut()
            .filter(|active| active.effect.registration_order == order)
            .ok_or("pause environment registration disappeared")?;
        active.binding.publish(
            view,
            slots,
            active.effect.view_mask,
            mask,
            active.effect.blend,
        )
    }
    pub(super) fn flush_environment_retirements(
        &mut self,
        view: u64,
        slots: &mut crate::rendering::environment_overrides::EnvironmentOverrides<
            Arc<crate::rendering::environment_resources::PreparedEnvironment>,
        >,
    ) {
        for (owner, slot) in self.retirements.drain(..) {
            if owner == view {
                slots.retire(slot);
            }
        }
    }
    pub(super) fn clear_environments(&mut self) -> Result<(), String> {
        self.closed = true;
        self.requested = None;
        self.remove()
    }
}

pub(super) fn update_pause_screen_fx(
    mut fx: ResMut<PauseScreenFx>,
    paused: Res<GamePaused>,
    clock: Res<ClockRuntime>,
    wheel: Option<Res<super::power_wheel::State>>,
) {
    let fx = &mut *fx;
    if fx.closed {
        return;
    }
    let result = (|| -> Result<(), String> {
        let entered = paused.0 && !fx.was_paused;
        fx.was_paused = paused.0;
        if !paused.0 && !wheel.as_ref().is_some_and(|wheel| wheel.screen_fx_active()) {
            fx.remove()?;
            fx.requested = None;
            return Ok(());
        }
        if entered {
            if !fx.start()? {
                fx.requested = Some(true);
            }
            return Ok(());
        }
        match fx.requested.take() {
            Some(true) => {
                if !fx.start()? {
                    fx.requested = Some(true);
                }
                return Ok(());
            }
            Some(false) => fx.remove()?,
            None => (),
        }
        if fx
            .active
            .as_ref()
            .is_some_and(|active| !active.effect.active)
        {
            fx.remove()?;
        }
        let Some(delta) = clock.clock().ok().and_then(|clock| clock.delta(0)) else {
            return Ok(());
        };
        if let Some(active) = &mut fx.active {
            active.time += delta * TIME_SCALE;
            active.effect.update(active.time, 1, &mut fx.seed)?;
            fx.weight = active.effect.blend;
        }
        Ok(())
    })();
    if let Err(error) = result {
        fx.error = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires installed game archives"]
    fn the_pause_screen_fx_blends_in_ui_pause() {
        let base = crate::assets::installation::game_base().unwrap();
        let catalog = Catalog::open(&base).unwrap();
        let fx = load(&catalog).unwrap();
        assert_eq!(fx.environment, "env/fx/ui_pause");
        let environment = environment_parameters(&catalog, &fx.environment).unwrap();
        assert!(environment.parameter("env/tonemapping/colorcube").is_some());
        assert!(environment
            .parameter("env/dof/settings/bokeh_size")
            .is_some());
        // This installed-data timing oracle exercises the actual kernel. Host
        // image readiness/native-pool admission require the full scene owner.
        let mut seed = 0;
        let offsets = sample_offsets(&fx.timings, 0., &mut seed);
        let mut effect = EnvironmentBlendEffect::new(fx.effect.clone(), offsets, 1, &mut seed).unwrap();
        let mut blend = |t: f32| {
            effect.update(t, 1, &mut seed).unwrap();
            effect.blend
        };
        // A 0.1 s fade-in, then the looping duration holds it.
        let (start, middle, held) = (blend(0.0), blend(0.05), blend(0.5));
        assert!(start < middle && middle < held, "{start} {middle} {held}");
        assert!((held - 1.0).abs() < 1e-3, "{held}");
    }
}
