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
use crate::gameplay::options::decl::{atom, list_items, read_edit, scope};
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
    let name = atom(&state, "m_screenFX").ok_or("the pause state has no m_screenFX")?.to_string();
    let edit = read_edit(&mut read, "fx", &name)?;
    let groups = scope(&edit, "m_groups").ok_or_else(|| format!("{name} has no m_groups"))?;
    for group in list_items(groups) {
        let crate::assets::entities::Value::Scope(fields) = group else { continue };
        let group = crate::assets::fx::environment_group(fields)?;
        if !group.enabled {
            continue;
        }
        if let Some(env) = group.environments.into_iter().find(|e| e.enabled && e.environment.is_some()) {
            return Ok(ScreenFx { timings: group.timings, effect: Arc::new(env.blend), environment: env.environment.unwrap() });
        }
    }
    Err(format!("{name} has no environment override"))
}

/// An environment decl's render parameters, following its inherit chain.
pub(super) fn environment_parameters(catalog: &Catalog, name: &str) -> Result<crate::assets::entities::EnvironmentRenderParameters, String> {
    let read = |name: &str| -> Result<Option<String>, String> {
        let resource = format!("generated/decls/env/{name}.decl");
        catalog.read(&resource).map(|b| b.map(|b| String::from_utf8_lossy(&b).into_owned())).map_err(|e| e.to_string())
    };
    let source = read(name)?.ok_or_else(|| format!("environment {name} is not installed"))?;
    crate::assets::entities::environment_render_parameters_with_inheritance(name, &source, read)
}

struct Active {
    effect: EnvironmentBlendEffect,
    time: f32,
}

#[derive(Resource)]
pub(super) struct PauseScreenFx {
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
        Self { fx, active: None, requested: None, was_paused: false, seed: 0, weight: 0. }
    }

    pub(super) fn environment(&self) -> Option<&str> {
        self.fx.as_ref().map(|f| f.environment.as_str())
    }

    /// `FUN_1407912e0` / `FUN_140791240`: a new FX instance at time zero.
    fn start(&mut self) {
        let Some(fx) = &self.fx else { return };
        let offsets = sample_offsets(&fx.timings, 0., &mut self.seed);
        match EnvironmentBlendEffect::new(fx.effect.clone(), offsets, 1, &mut self.seed) {
            Ok(effect) => self.active = Some(Active { effect, time: 0. }),
            Err(e) => warn!("Pause screen FX: {e}"),
        }
        self.weight = 0.;
    }

    /// `FUN_140791370`: the FX is stopped and removed from its entity.
    fn remove(&mut self) {
        self.active = None;
        self.weight = 0.;
    }
}

pub(super) fn update_pause_screen_fx(mut fx: ResMut<PauseScreenFx>, paused: Res<GamePaused>, clock: Res<ClockRuntime>) {
    let fx = &mut *fx;
    let entered = paused.0 && !fx.was_paused;
    fx.was_paused = paused.0;
    if !paused.0 {
        fx.remove();
        fx.requested = None;
        return;
    }
    if entered {
        fx.start();
        return;
    }
    match fx.requested.take() {
        // `EnableUIPostFX` starts it on the next FX update, which then does
        // not advance it.
        Some(true) => {
            fx.start();
            return;
        }
        Some(false) => fx.remove(),
        None => {}
    }
    let Some(delta) = clock.clock().ok().and_then(|c| c.delta(0)) else { return };
    let seed = &mut fx.seed;
    if let Some(active) = &mut fx.active {
        active.time += delta * TIME_SCALE;
        match active.effect.update(active.time, 1, seed) {
            Ok(()) => fx.weight = active.effect.blend,
            Err(e) => warn!("Pause screen FX: {e}"),
        }
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
        assert!(environment.parameter("env/dof/settings/bokeh_size").is_some());
        let mut state = PauseScreenFx::new(Some(fx));
        state.start();
        let seed = &mut state.seed;
        let active = state.active.as_mut().unwrap();
        let mut blend = |t: f32| {
            active.effect.update(t, 1, seed).unwrap();
            active.effect.blend
        };
        // A 0.1 s fade-in, then the looping duration holds it.
        let (start, middle, held) = (blend(0.0), blend(0.05), blend(0.5));
        assert!(start < middle && middle < held, "{start} {middle} {held}");
        assert!((held - 1.0).abs() < 1e-3, "{held}");
    }
}
