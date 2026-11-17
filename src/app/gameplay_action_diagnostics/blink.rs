//! Windowless production-input Blink sequence. No state/target/time override.
//! Assertions use installed power2, state63/74/12 and actual FX publications;
//! the render owner separately records its real fullscreen draw receipt.
use super::{Input, binding, input_for, player, send};
use super::super as app;
use bevy::prelude::*;
use crate::gameplay::{player::states::StateId, powers::Phase};
use std::time::{Duration, Instant};

const TOP: &str = "models/effects/gameplay/powers/blink/blink_target_01.fx";
const BOTTOM: &str = "models/effects/gameplay/powers/blink/blink_target_02.fx";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage { Select, AimCancel, Cancel, Rearm, AimCast, Cast, Recharge, Holster, Redraw, Done }

pub(super) struct Observation {
    stage: Stage,
    since: Instant,
    release: Option<Input>,
    held: Option<Input>,
    use_input: Input,
    selection_input: Input,
    holster_input: Input,
    owners: [Option<String>; 2],
    aiming_ready: Option<Instant>,
    ledge_reported: bool,
    cancel_seen: bool,
    blink_seen: bool,
    mantle_seen: bool,
    cast_start: Vec3,
    mana_before: f32,
    mana_low: f32,
    spent: bool,
    regen_event: Option<u32>,
    regen_admitted: bool,
    regen_source_ready: bool,
    regen_callback: bool,
    audio_before: usize,
}

impl Observation {
    pub(super) fn begin(world: &mut World) -> Result<Self, String> {
        let inventory = world.get_resource::<app::interaction_focus::DiscoveredDocuments>()
            .ok_or("Blink diagnostic inventory absent")?;
        let definitions = world.get_resource::<app::powers::Powers>()
            .ok_or("Blink diagnostic power definitions absent")?;
        let item = definitions.definitions.0.iter().find(|power|
            crate::assets::power_journal::power_id(&power.power_id) == Some(2)
                && inventory.quantity(&power.item) > 0)
            .ok_or("Blink scenario requires actually owned power2 in the loaded character inventory")?;
        let action = (0..10).find(|&index| inventory.wheel_shortcut_pair(index).0 == Some(item.item.as_str()))
            .map(|index| 25 + index as i32)
            .ok_or("owned Blink has no actual physical shortcut in the loaded controller set")?;
        let item = item.item.clone();
        let selection = input_for(action)?;
        let use_input = input_for(20)?;
        let holster_input = input_for(22)?;
        if binding(Input::Key(KeyCode::KeyF)).is_none() {
            return Err("F has no actual gameplay binding; cannot verify reported F cancellation".into());
        }
        let mana = player(world)?.mana();
        let regen_event = player(world)?.mana_regen_sound_event();
        let audio_before = app::audio::observation::snapshot(world).admissions.len();
        eprintln!("ACTION_SMOKE_BLINK_BEGIN owned_item={item} shortcut_action={action} selection={selection:?} use_input={use_input:?} cancel_physical=KeyF cancel_action={:?} mana={mana} regen_event={regen_event:?} no_teleport=true no_state_or_target_override=true no_fake_ACK=true", binding(Input::Key(KeyCode::KeyF)));
        send(world, selection, true)?;
        Ok(Self { stage: Stage::Select, since: Instant::now(), release: Some(selection), held: None,
            use_input, selection_input: selection, holster_input, owners: [None, None], aiming_ready: None, ledge_reported: false, cancel_seen: false,
            blink_seen: false, mantle_seen: false, cast_start: Vec3::ZERO, mana_before: mana,
            mana_low: mana, spent: false, regen_event, regen_admitted: false,
            regen_source_ready: false, regen_callback: false, audio_before })
    }

    fn stage(&mut self, next: Stage, world: &World) -> Result<(), String> {
        let p = player(world)?;
        eprintln!("ACTION_SMOKE_BLINK_STAGE from={:?} next={next:?} elapsed={} states={:?} phase={:?} feet={:?} camera={:?} mana={} top_owner={:?} bottom_owner={:?}",
            self.stage, self.since.elapsed().as_secs_f64(), p.active_player_states(),
            p.selected_power().map(|cast| (cast.settings.id, cast.phase())), p.feet(),
            p.published_camera_frame(), p.mana(), self.owners[0], self.owners[1]);
        self.stage = next; self.since = Instant::now(); self.aiming_ready = None; Ok(())
    }
    fn hold_use(&mut self, world: &mut World) -> Result<(), String> {
        send(world, self.use_input, true)?; self.held = Some(self.use_input); Ok(())
    }
    fn release_use(&mut self, world: &mut World) -> Result<(), String> {
        if let Some(input) = self.held.take() { send(world, input, false)?; } Ok(())
    }
    fn aim_ready(&mut self, world: &World) -> Result<bool, String> {
        let p = player(world)?;
        if !p.machine.is_active(StateId::USE_POWER)
            || !p.selected_power().is_some_and(|cast| cast.settings.id == 2 && cast.phase() == Phase::Aiming) { return Ok(false); }
        let Some(actions) = world.get_resource::<app::player_actions::PlayerActions>() else { return Ok(false) };
        let Some(target) = actions.blink.as_ref().and_then(|owner| owner.blink.target.as_ref()) else { return Ok(false) };
        if target.mantle.is_some() {
            // This sequence must exercise the authored top marker's FSdisto.
            // A real ledge target is retained as evidence, never forced away.
            if !self.ledge_reported {
                eprintln!("ACTION_SMOKE_BLINK_LEDGE target={target:?} top_marker_required_for_GPU_case=true");
                self.ledge_reported = true;
            }
            return Ok(false);
        }
        if target.top_fx.is_none() || target.bottom_fx.is_none() { return Ok(false); }
        if let Some(dynamic) = world.get_resource::<crate::rendering::bevy_backend::world_effects::DynamicEffects>() {
            for entry in dynamic.0.iter() {
                if entry.effect == TOP { self.owners[0] = Some(entry.owner.clone()); }
                if entry.effect == BOTTOM { self.owners[1] = Some(entry.owner.clone()); }
            }
        }
        let Some(sim) = world.get_resource::<crate::rendering::bevy_backend::camera_scene::FxSimulation>() else { return Ok(false) };
        let ready = self.owners.iter().all(|owner| owner.as_deref().is_some_and(|owner| {
            let snapshot = sim.observe_fx_owner(owner);
            !snapshot.handles.is_empty() && snapshot.pending_starts == 0 && snapshot.particles > 0
        }));
        let distortion = sim.observe_screen_distortion();
        if !ready || !distortion.wobble { return Ok(false); }
        let first = self.aiming_ready.get_or_insert_with(|| {
            eprintln!("ACTION_SMOKE_PASS check=blink_actual_aim_markers_and_FSD_input target={target:?} body={:?} camera={:?} distortion={distortion:?} GPU_pixels_not_claimed=true", p.feet(), p.published_camera_frame());
            Instant::now()
        });
        // Let the normal render thread consume multiple actual publications.
        // This waits real wall time; it never advances a gameplay clock.
        Ok(first.elapsed() >= Duration::from_millis(500))
    }
    fn helpers_retired(&self, world: &World) -> bool {
        world.get_resource::<crate::rendering::bevy_backend::camera_scene::FxSimulation>()
            .is_some_and(|sim| self.owners.iter().all(|owner| owner.as_deref().is_some_and(|owner| {
                let snapshot = sim.observe_fx_owner(owner);
                snapshot.handles.is_empty() && snapshot.pending_starts == 0
            })))
    }

    pub(super) fn poll(&mut self, world: &mut World) -> Result<bool, String> {
        let p = player(world)?;
        self.cancel_seen |= p.machine.is_active(StateId::CANCEL_ACTION);
        self.blink_seen |= p.machine.is_active(StateId::BLINKING);
        self.mantle_seen |= p.machine.is_active(StateId::MANTLE);
        if matches!(self.stage, Stage::Cast | Stage::Recharge) {
            self.spent |= p.mana() < self.mana_before;
            self.mana_low = self.mana_low.min(p.mana());
        }
        if let Some(input) = self.release.take() { send(world, input, false)?; return Ok(false); }
        if self.since.elapsed() > Duration::from_secs(90) {
            let mut snapshot = String::new(); player(world)?.write_combat_diagnostics(&mut snapshot);
            return Err(format!("Blink actual-input timeout stage={:?} cancel74={} blink12={} spent={} snapshot={snapshot}", self.stage, self.cancel_seen, self.blink_seen, self.spent));
        }
        let stage = self.stage;
        match stage {
            Stage::Select if player(world)?.selected_power_hud_status() == Some((2, true))
                && player(world)?.equipment_action_status(0).is_some_and(|(_, equipped, transitioning)| equipped && !transitioning) => {
                self.mana_before = player(world)?.mana();
                self.hold_use(world)?; self.stage(Stage::AimCancel, world)?;
            }
            Stage::AimCancel if self.aim_ready(world)? => {
                send(world, Input::Key(KeyCode::KeyF), true)?;
                self.release = Some(Input::Key(KeyCode::KeyF)); self.stage(Stage::Cancel, world)?;
            }
            Stage::Cancel if !player(world)?.power_cast_active(2)
                && !player(world)?.machine.is_active(StateId::USE_POWER) && self.helpers_retired(world) => {
                if !self.cancel_seen || self.blink_seen || player(world)?.mana() < self.mana_before {
                    return Err(format!("F cancel failed native74/no_cast/no_mana_spend contract: cancel74={} blink12={} before={} after={}", self.cancel_seen, self.blink_seen, self.mana_before, player(world)?.mana()));
                }
                self.release_use(world)?;
                eprintln!("ACTION_SMOKE_PASS check=blink_F_cancel74 no_blink12=true no_mana_spend=true helper_tails_retired=true");
                self.owners = [None, None]; self.stage(Stage::Rearm, world)?;
            }
            Stage::Rearm if player(world)?.selected_power_hud_status() == Some((2, true)) => {
                self.hold_use(world)?; self.stage(Stage::AimCast, world)?;
            }
            Stage::AimCast if self.aim_ready(world)? => {
                self.cast_start = player(world)?.feet(); self.mana_before = player(world)?.mana(); self.mana_low = self.mana_before;
                self.release_use(world)?; self.stage(Stage::Cast, world)?;
            }
            Stage::Cast if self.blink_seen && self.spent
                && !player(world)?.machine.is_active(StateId::BLINKING)
                && !player(world)?.power_cast_active(2) && self.helpers_retired(world) => {
                let p = player(world)?;
                if p.feet() == self.cast_start { return Err("Blink12 exited and spent mana without moving the actual player proxy".into()); }
                eprintln!("ACTION_SMOKE_PASS check=blink_release_cast12_and_mana actual_start={:?} actual_end={:?} camera={:?} mana_before={} mana_low={} helpers_retired=true", self.cast_start, p.feet(), p.published_camera_frame(), self.mana_before, self.mana_low);
                if !self.mantle_seen {
                    eprintln!("ACTION_SMOKE_NOT_COVERED check=blink_to_mantle reason=normal_authored_spawn_target_has_no_admitted_ledge needs_user_ledge_case=true");
                }
                self.stage(Stage::Recharge, world)?;
            }
            Stage::Recharge => {
                let audio = app::audio::observation::snapshot(world);
                if let Some(event) = self.regen_event {
                    for admission in audio.admissions.iter().skip(self.audio_before).filter(|admission| admission.event == event) {
                        if let Some(error) = &admission.error { return Err(format!("actual mana regeneration event failed: {error}")); }
                        self.regen_admitted = true;
                    }
                    for callback in audio.callbacks.iter().filter(|callback| callback.declaration == event) {
                        self.regen_source_ready |= callback.kind == "SourceReady";
                        self.regen_callback = true;
                    }
                }
                if player(world)?.mana() > self.mana_low && (self.regen_event.is_none() || self.regen_admitted) {
                    eprintln!("ACTION_SMOKE_PASS check=blink_mana_regenerates current={} low={} actual_regen_event={:?} admitted={} source_ready_callback={} any_callback={} audible_line_not_claimed=true", player(world)?.mana(), self.mana_low, self.regen_event, self.regen_admitted, self.regen_source_ready, self.regen_callback);
                    send(world, self.holster_input, true)?;
                    self.held = Some(self.holster_input);
                    self.stage(Stage::Holster, world)?;
                }
            }
            Stage::Holster => {
                let p = player(world)?;
                if (0..2).all(|side| p.equipment_action_status(side)
                    .is_some_and(|(_, equipped, changing)| !equipped && !changing)) {
                    self.release_use(world)?;
                    eprintln!("ACTION_SMOKE_PASS check=power_and_sword_actual_hold_undraw both_hands_unequipped=true");
                    send(world, self.selection_input, true)?;
                    self.release = Some(self.selection_input);
                    self.stage(Stage::Redraw, world)?;
                }
            }
            Stage::Redraw if player(world)?.selected_power_hud_status() == Some((2, true))
                && player(world)?.equipment_action_status(0).is_some_and(|(_, equipped, changing)| equipped && !changing) => {
                eprintln!("ACTION_SMOKE_PASS check=power_actual_shortcut_redraw current_power=2 equipped=true no_pending_switch_or_holster=true");
                self.stage(Stage::Done, world)?;
            }
            Stage::Done => {
                eprintln!("ACTION_SMOKE_PASS check=blink_input_sequence_complete GPU_record_receipt_required_separately=ACTION_SMOKE_FSDISTO_RENDER pixels_verified=false exit=ordinary_ExitDrain");
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
}
