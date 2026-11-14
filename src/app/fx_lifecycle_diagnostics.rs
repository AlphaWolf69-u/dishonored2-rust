//! Windowless observation of the normal FX request/advance/retirement owners.
//! The caller advances the real App and its clocks. This module never creates
//! a simulation, advances time, changes visibility, or acknowledges audio/GPU work.
use bevy::prelude::World;
use std::sync::Weak;
use crate::rendering::{bevy_backend::camera_scene::FxSimulation, particles::world_sources::WorldFxSources};
use super::usable_effects::{EffectOwner, EffectRequests, UsableEffects};

enum Phase { Start, Overlap, ViewGap, Restart, Drain, Complete }
struct Scenario {
    path: String,
    first: String,
    second: String,
    source: Option<Weak<WorldFxSources>>,
    gap_handles: Vec<u32>,
    gap_frame: Option<u32>,
    phase: Phase,
}
/// Retains only weak resource observations and request names. Dropping this
/// record cannot keep a stopped effect or its decoded payload alive.
pub(super) struct Observation {
    scenarios: Vec<Scenario>,
    next: usize,
    view_gap: bool,
}
impl Observation {
    pub(super) fn begin(world: &mut World, paths: &[String]) -> Result<Self, String> {
        if paths.is_empty() { return Err("active FX observation requires installed effect paths".into()); }
        if !world.contains_resource::<FxSimulation>() || !world.contains_resource::<EffectRequests>()
            || !world.contains_resource::<UsableEffects>() {
            return Err("active FX observation requires the published normal gameplay owners".into());
        }
        let scenarios = paths.iter().enumerate().map(|(index, path)| Scenario {
            path: path.clone(), first: format!("lifecycle-{index}-a"), second: format!("lifecycle-{index}-b"),
            source: None, gap_handles: Vec::new(), gap_frame: None, phase: Phase::Start,
        }).collect::<Vec<_>>();
        world.resource_mut::<EffectRequests>().start_player(&scenarios[0].first, &scenarios[0].path);
        eprintln!("FX_ACTIVE_LIFETIME phase=begin scenarios={} pools={:?}", scenarios.len(), world.resource::<FxSimulation>().observe_fx_pools());
        Ok(Self { scenarios, next: 0, view_gap: false })
    }
    pub(super) fn needs_view_gap(&self) -> bool { self.view_gap }
    pub(super) fn pending_description(&self) -> String {
        self.scenarios.get(self.next).map_or_else(|| "complete".into(), |scenario| {
            let phase = match scenario.phase { Phase::Start => "start", Phase::Overlap => "overlap",
                Phase::ViewGap => "view-gap", Phase::Restart => "restart-request", Phase::Drain => "tail-drain", Phase::Complete => "complete" };
            format!("{}: {phase}", scenario.path)
        })
    }
    /// Called AFTER the adapter genuinely withdraws the active Camera and runs
    /// the ordinary App. Missing views must not destroy simulation/resource owners.
    pub(super) fn observe_view_gap(&mut self, world: &World) -> Result<(), String> {
        let scenario = self.scenarios.get_mut(self.next).ok_or("FX gap scenario absent")?;
        if !self.view_gap || !matches!(scenario.phase, Phase::ViewGap) { return Err("FX view gap was not requested".into()); }
        let sample = world.get_resource::<FxSimulation>().ok_or("FX simulation dropped with its view")?
            .observe_fx_owner(&format!("player:{}", scenario.first));
        let retained = scenario.source.as_ref().is_some_and(|source| source.strong_count() != 0);
        if sample.handles != scenario.gap_handles || !retained {
            return Err(format!("view withdrawal destroyed active FX ownership: {} handles={:?} leases={retained}", scenario.path, sample.handles));
        }
        eprintln!("FX_ACTIVE_LIFETIME phase=view-gap effect={} before_frame={:?} after_frame={:?} handles={:?} source_live={retained}",
            scenario.path, scenario.gap_frame, sample.frame, sample.handles);
        self.view_gap = false;
        // The caller restores its Camera before the next ordinary App frame.
        scenario.phase = Phase::Restart;
        Ok(())
    }
    /// One observation after an actual App.update. true requests canonical map
    /// shutdown; persistent decal records remain owned until that actual shutdown.
    pub(super) fn poll(&mut self, world: &mut World) -> Result<bool, String> {
        let Some(scenario) = self.scenarios.get_mut(self.next) else { return Ok(true); };
        let first_key = format!("player:{}", scenario.first);
        let second_key = format!("player:{}", scenario.second);
        let simulation = world.get_resource::<FxSimulation>().ok_or("active FX simulation is absent")?;
        let first = simulation.observe_fx_owner(&first_key);
        let second = simulation.observe_fx_owner(&second_key);
        let acquired = world.get_resource::<UsableEffects>().ok_or("active FX acquisition owner is absent")?;
        let ready = acquired.ready.get(&EffectOwner::Player(scenario.first.clone()));
        if let Some(source) = ready {
            if let Some(Err(error)) = source.effects.get(&scenario.path) { return Err(error.clone()); }
        }
        match scenario.phase {
            Phase::Start => {
                if first.handles.is_empty() || first.pending_starts != 0 { return Ok(false); }
                let source = ready.ok_or("playing FX has no request source owner")?;
                scenario.source = Some(std::sync::Arc::downgrade(source));
                eprintln!("FX_ACTIVE_LIFETIME phase=started effect={} snapshot={first:?}", scenario.path);
                world.resource_mut::<EffectRequests>().start_player(&scenario.second, &scenario.path);
                scenario.phase = Phase::Overlap;
            },
            Phase::Overlap => {
                if second.handles.is_empty() || second.pending_starts != 0 { return Ok(false); }
                let other = acquired.ready.get(&EffectOwner::Player(scenario.second.clone())).ok_or("overlapping FX source owner absent")?;
                let shared = scenario.source.as_ref().is_some_and(|source| source.ptr_eq(&std::sync::Arc::downgrade(other)));
                if !shared { return Err("same-path overlapping FX did not reuse its immutable source owner".into()); }
                if first.handles.is_empty() { return Err(format!("{} ended before its view-gap observation; select an authored looping effect", scenario.path)); }
                scenario.gap_handles = first.handles;
                scenario.gap_frame = first.frame;
                eprintln!("FX_ACTIVE_LIFETIME phase=overlap effect={} source_shared={shared} second={second:?}", scenario.path);
                scenario.phase = Phase::ViewGap;
                self.view_gap = true;
            },
            Phase::ViewGap => {},
            Phase::Restart => {
                // A real Stop followed by Start for the same owner leaves the
                // old stopped instance's tails independent of its new revision.
                let mut requests = world.resource_mut::<EffectRequests>();
                requests.stop_player(&scenario.first);
                requests.start_player(&scenario.first, &scenario.path);
                requests.stop_player(&scenario.second);
                eprintln!("FX_ACTIVE_LIFETIME phase=restart-request effect={} old_handles={:?}", scenario.path, scenario.gap_handles);
                scenario.phase = Phase::Drain;
                // The new revision has not been consumed yet. The next frame
                // must observe its new handle before issuing the final Stop.
            },
            Phase::Drain => {
                let restarted = first.handles.iter().any(|handle| !scenario.gap_handles.contains(handle));
                if !scenario.gap_handles.is_empty() {
                    if !restarted { return Ok(false); }
                    let source = ready.ok_or("restarted FX source owner absent")?;
                    let shared = scenario.source.as_ref().is_some_and(|old| old.ptr_eq(&std::sync::Arc::downgrade(source)));
                    if !shared { return Err("warm restart replaced the retained immutable FX source owner".into()); }
                    eprintln!("FX_ACTIVE_LIFETIME phase=restarted effect={} snapshot={first:?} source_shared={shared}", scenario.path);
                    world.resource_mut::<EffectRequests>().stop_player(&scenario.first);
                    scenario.gap_handles.clear();
                    return Ok(false);
                }
                if ready.is_some() || acquired.ready.contains_key(&EffectOwner::Player(scenario.second.clone())) { return Ok(false); }
                let drained = first.handles.is_empty() && second.handles.is_empty();
                let decal_owned = (first.handles.is_empty() || first.only_decal_tails())
                    && (second.handles.is_empty() || second.only_decal_tails());
                if !drained && !decal_owned { return Ok(false); }
                eprintln!("FX_ACTIVE_LIFETIME phase=stopped effect={} naturally_drained={drained} decal_owned_until_map_clear={} first={first:?} second={second:?} source_live={}",
                    scenario.path, !drained, scenario.source.as_ref().is_some_and(|source| source.strong_count() != 0));
                scenario.phase = Phase::Complete;
                self.next += 1;
                if let Some(next) = self.scenarios.get(self.next) {
                    world.resource_mut::<EffectRequests>().start_player(&next.first, &next.path);
                }
            },
            Phase::Complete => {},
        }
        Ok(self.next == self.scenarios.len())
    }
    /// Called only after the actual canonical LevelLifetime/session exit drain.
    /// No forced TTL or synthetic acknowledgement is used to obtain this result.
    pub(super) fn observe_shutdown(&self, world: &World) -> Result<(), String> {
        let pools = world.get_resource::<FxSimulation>().map(FxSimulation::observe_fx_pools);
        if pools.is_some_and(|counts| counts != (0, 0, 0, 0, 0)) { return Err(format!("actual shutdown retained active FX pools: {pools:?}")); }
        let retained = self.scenarios.iter().filter(|scenario| scenario.source.as_ref().is_some_and(|source| source.strong_count() != 0)).count();
        eprintln!("FX_ACTIVE_LIFETIME phase=shutdown pools={pools:?} source_leases={retained}");
        if retained != 0 { return Err("actual shutdown retained observed dynamic FX source owners".into()); }
        Ok(())
    }
}
