//! Actual F8 water setup, then production dive/aim/cast/rearm input only.
//! Restore once; never write a target, mode, clock, state or cast position.
use super::*;
use crate::gameplay::sound::Emitter;
use crate::gameplay::{game_clock_runtime::ClockRuntime, powers::Phase};

const FEET: Vec3 = Vec3::new(-29.932283, 182.57669, -1.4056311);
const FORWARD: Vec3 = Vec3::new(-0.8852807, -0.024919145, 0.46438906);
// Existing actual EotW pier/start position, used solely for mouse steering.
const PIER: Vec3 = Vec3::new(-31.830004, 188.40999, 1.2120594);
// Installed Init/common sound-bank state/action identities; diagnostic only.
const UNDERWATER_GROUP: u32 = 0xe58b68e3;
const UNDERWATER_VALUE: u32 = 0x7dd27190;
const OUTSIDE_BUS: u32 = 1973600711;
const SURFACE_EVENT: u32 = 1510240915;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Water,
    Select,
    Surface,
    Orient,
    Dive,
    Aim,
    Flight,
    Recovery,
    Rearm,
    Cancel,
    Done,
}

#[derive(Clone, Copy, Debug)]
struct Stamp {
    wall: f64,
    clock: f64,
    frame: u32,
}
impl Stamp {
    fn elapsed(self, earlier: Self) -> (f64, f64, u32) {
        (
            self.wall - earlier.wall,
            self.clock - earlier.clock,
            self.frame.wrapping_sub(earlier.frame),
        )
    }
}

pub(super) struct Observation {
    stage: Stage,
    start: Instant,
    since: Instant,
    release: Option<Input>,
    diving: bool,
    rising: bool,
    using: bool,
    selection: Input,
    up: Input,
    down: Input,
    use_input: Input,
    cancel: Input,
    cast_start: Vec3,
    mana_before: f32,
    mana_spent: bool,
    underwater_seen: bool,
    blink_seen: bool,
    blink_in_water: bool,
    aim_observed: bool,
    water_exit: Option<Stamp>,
    exit_during_blink: bool,
    released: Option<Stamp>,
    entered: Option<Stamp>,
    finished: Option<Stamp>,
    cooldown_ready: Option<Stamp>,
    rearm_input: Option<Stamp>,
    last_water: bool,
    last_blink: bool,
    breath_start: Option<u32>,
    audio_before: usize,
    breath_emitter: Emitter,
    breath_object: String,
    breath_voice: Option<u32>,
    breath_source: bool,
    mix_baseline: Option<[f32; 6]>,
    mix_underwater: Option<[f32; 6]>,
    mix_surface: bool,
    mix_value: Option<u32>,
    audio_error: Option<String>,
    surface_emitters: Vec<Emitter>,
    surface_fx_source: bool,
}

impl Observation {
    pub(super) fn begin(world: &mut World) -> Result<Self, String> {
        let inventory = world
            .get_resource::<DiscoveredDocuments>()
            .ok_or("water Blink inventory absent")?;
        let definitions = world
            .get_resource::<super::super::powers::Powers>()
            .ok_or("water Blink definitions absent")?;
        let item = definitions
            .definitions
            .0
            .iter()
            .find(|power| {
                crate::assets::power_journal::power_id(&power.power_id) == Some(2)
                    && inventory.quantity(&power.item) > 0
            })
            .ok_or("water Blink is not genuinely owned")?;
        let action = (0..10)
            .find(|&i| inventory.wheel_shortcut_pair(i).0 == Some(item.item.as_str()))
            .map(|i| 25 + i as i32)
            .ok_or("owned water Blink has no installed shortcut")?;
        let selection = input_for(action)?;
        let up = input_for(crate::gameplay::player::controller::ACTION_JUMP)?;
        let down = input_for(crate::gameplay::player::controller::ACTION_CROUCH)?;
        let use_input = input_for(20)?;
        let cancel = input_for(22)?;
        let breath_start = player(world)?
            .breath_audio()
            .and_then(|audio| audio.start.as_ref())
            .map(|event| event.id);
        let breath_emitter = Emitter::PlayerJoint {
            entity: None,
            part: 1,
            joint: player(world)?.body.skeleton.camera as u16,
        };
        let breath_object = format!("{breath_emitter:?}");
        world
            .resource_mut::<PlayerState>()
            .player
            .as_mut()
            .ok_or("water Blink Player absent")?
            .teleport(FEET, FORWARD.y.atan2(FORWARD.x));
        eprintln!(
            "ACTION_SMOKE_BLINK_WATER_SETUP source=collision-1794201780722 feet={FEET:?} captured_forward={FORWARD:?} steer_landmark={PIER:?} setup_restore_once=true subsequent_motion=physical_inputs no_target_or_state_or_clock_override=true breath_start={breath_start:?}"
        );
        Ok(Self {
            stage: Stage::Water,
            start: Instant::now(),
            since: Instant::now(),
            release: None,
            diving: false,
            rising: false,
            using: false,
            selection,
            up,
            down,
            use_input,
            cancel,
            cast_start: FEET,
            mana_before: 0.,
            mana_spent: false,
            underwater_seen: false,
            blink_seen: false,
            blink_in_water: false,
            aim_observed: false,
            water_exit: None,
            exit_during_blink: false,
            released: None,
            entered: None,
            finished: None,
            cooldown_ready: None,
            rearm_input: None,
            last_water: false,
            last_blink: false,
            breath_start,
            audio_before: 0,
            breath_emitter,
            breath_object,
            breath_voice: None,
            breath_source: false,
            mix_baseline: None,
            mix_underwater: None,
            mix_surface: false,
            mix_value: None,
            audio_error: None,
            surface_emitters: Vec::new(),
            surface_fx_source: false,
        })
    }

    fn stamp(&self, world: &World) -> Result<Stamp, String> {
        Ok(Stamp {
            wall: self.start.elapsed().as_secs_f64(),
            clock: world
                .resource::<ClockRuntime>()
                .player_time()
                .map_err(str::to_owned)?,
            frame: world.resource::<bevy::diagnostic::FrameCount>().0,
        })
    }
    fn stage(&mut self, next: Stage, world: &World) -> Result<(), String> {
        let p = player(world)?;
        eprintln!(
            "ACTION_SMOKE_BLINK_WATER_STAGE from={:?} next={next:?} stamp={:?} feet={:?} swimming={} underwater={} breath={} states={:?} cast={:?} mana={}",
            self.stage,
            self.stamp(world)?,
            p.feet(),
            p.is_swimming(),
            p.is_underwater(),
            p.breath(),
            p.active_player_states(),
            p.selected_power()
                .map(|cast| (cast.settings.id, cast.phase())),
            p.mana()
        );
        self.stage = next;
        self.since = Instant::now();
        Ok(())
    }
    fn steer(world: &mut World) -> Result<bool, String> {
        let (eye, axis) = player(world)?.published_camera_frame();
        let desired = PIER - eye;
        let yaw = desired.y.atan2(desired.x) - axis.x_axis.y.atan2(axis.x_axis.x);
        let yaw = yaw.sin().atan2(yaw.cos());
        let pitch = -desired.z.atan2(desired.truncate().length())
            + axis.x_axis.z.atan2(axis.x_axis.truncate().length());
        world.write_message(bevy::input::mouse::MouseMotion {
            delta: Vec2::new((-yaw * 100.).clamp(-8., 8.), (pitch * 100.).clamp(-8., 8.)),
        });
        // Setup steering convergence only; this never admits a Blink target.
        Ok(yaw.abs() < 0.01 && pitch.abs() < 0.01)
    }
    fn mix(&mut self, world: &World) -> Result<(), String> {
        //Measure the actual dive/cast, not the setup surface recovery.
        if matches!(self.stage, Stage::Water | Stage::Select | Stage::Surface) {
            return Ok(());
        }
        if self.mix_surface {
            return Ok(());
        }
        let snapshot = super::super::audio::observation::state_snapshot(world, UNDERWATER_GROUP)?;
        let published = snapshot
            .properties
            .iter()
            .find(|(node, _)| *node == OUTSIDE_BUS)
            .map(|(_, values)| values);
        if self.mix_value != Some(snapshot.value) {
            self.mix_value = Some(snapshot.value);
            eprintln!(
                "ACTION_SMOKE_BLINK_WATER_MIX_OBSERVED group={UNDERWATER_GROUP:#010x} value={:#010x} node={OUTSIDE_BUS} published={published:?} underwater={} missing_published_node_is_not_mix_proof=true",
                snapshot.value,
                player(world)?.is_underwater()
            );
        }
        let Some(values) = published else {
            return Ok(());
        };
        if self.mix_baseline.is_none() && !player(world)?.is_underwater() && snapshot.value == 0 {
            self.mix_baseline = Some(*values);
            eprintln!(
                "ACTION_SMOKE_BLINK_WATER_MIX_BASELINE group={UNDERWATER_GROUP:#010x} value={} node={OUTSIDE_BUS} published={values:?} read_only_no_curve_advance=true",
                snapshot.value
            );
        }
        if let Some(baseline) = self.mix_baseline {
            if self.mix_underwater.is_none()
                && player(world)?.is_underwater()
                && snapshot.value == UNDERWATER_VALUE
                && values[3] > baseline[3]
            {
                self.mix_underwater = Some(*values);
                eprintln!(
                    "ACTION_SMOKE_PASS check=blink_water_actual_underwater_mix group={UNDERWATER_GROUP:#010x} value={UNDERWATER_VALUE:#010x} node={OUTSIDE_BUS} baseline={baseline:?} published={values:?} authored_lpf_target=57 no_heard_mix_claim=true"
                );
            }
            if self.mix_underwater.is_some()
                && !player(world)?.is_underwater()
                && snapshot.value == 0
                && values[3] <= baseline[3]
            {
                self.mix_surface = true;
                eprintln!(
                    "ACTION_SMOKE_PASS check=blink_water_actual_surface_mix group={UNDERWATER_GROUP:#010x} value=0 node={OUTSIDE_BUS} baseline={baseline:?} underwater={:?} published={values:?} no_heard_mix_claim=true",
                    self.mix_underwater
                );
            }
        }
        Ok(())
    }
    fn surface_emitters(&mut self, world: &World) -> Result<(), String> {
        if self.surface_fx_source || !player(world)?.is_underwater() {
            return Ok(());
        }
        let Some(effects) =
            world.get_resource::<crate::rendering::bevy_backend::world_effects::DynamicEffects>()
        else {
            return Ok(());
        };
        let Some(simulation) =
            world.get_resource::<crate::rendering::bevy_backend::camera_scene::FxSimulation>()
        else {
            return Ok(());
        };
        for entry in effects.0.iter().filter(|entry| {
            matches!(
                &entry.target,
                crate::gameplay::event_graph::EntityReference::Player
            )
        }) {
            let Some(definition) = entry.sources.effects.get(&entry.effect) else {
                continue;
            };
            let definition = definition.as_ref().map_err(Clone::clone)?;
            if !definition
                .groups
                .iter()
                .filter(|group| group.asset.enabled)
                .flat_map(|group| &group.sounds)
                .any(|sound| {
                    sound
                        .stop
                        .as_ref()
                        .is_some_and(|event| event.declaration.id == SURFACE_EVENT)
                })
            {
                continue;
            }
            for emitter in simulation.observe_fx_owner_audio(&entry.owner)? {
                if self.surface_emitters.contains(&emitter) {
                    continue;
                }
                if self.surface_emitters.len() == 512 {
                    return Err(
                        "water sound observation exceeds actual512 sound-family identities".into(),
                    );
                }
                eprintln!(
                    "ACTION_SMOKE_BLINK_WATER_SURFACE_OWNER owner={} effect={} actual_sound_emitter={emitter:?} acquired_stop_event={SURFACE_EVENT}",
                    entry.owner, entry.effect
                );
                self.surface_emitters.push(emitter);
            }
        }
        Ok(())
    }
    fn audio(&mut self, world: &mut World) -> Result<(), String> {
        if self.water_exit.is_none()
            || (self.surface_fx_source && (self.breath_start.is_none() || self.breath_source))
        {
            return Ok(());
        }
        let audio = super::super::audio::observation::snapshot(world);
        if !self.surface_fx_source {
            if let Some(sounds) =
                world.get_resource::<crate::rendering::bevy_backend::world_sounds::WorldSounds>()
            {
                let sounds = sounds.lock();
                for source in &audio.source_samples {
                    if !self.surface_emitters.contains(&source.object) {
                        continue;
                    }
                    let Some(engine_event) = sounds
                        .declarations
                        .get(source.event as usize)
                        .and_then(Option::as_ref)
                        .map(|event| event.declaration.id)
                    else {
                        continue;
                    };
                    if engine_event != SURFACE_EVENT {
                        continue;
                    }
                    let object = format!("{:?}", source.object);
                    let Some(admission) =
                        audio.admissions.iter().skip(self.audio_before).find(|row| {
                            row.playing_id == source.playing_id
                                && row.object == object
                                && row.event == engine_event
                        })
                    else {
                        continue;
                    };
                    if let Some(error) = &admission.error {
                        return Err(format!("actual surfacing sound admission failed: {error}"));
                    }
                    if !audio.source_processed_ids.contains(&source.playing_id)
                        || source.frames == 0
                        || source.decoded_peak <= 0.
                        || source.dry_peak <= 0.
                    {
                        continue;
                    }
                    self.surface_fx_source = true;
                    eprintln!(
                        "ACTION_SMOKE_PASS check=blink_water_surface_fx_actual_source admitted_engine_event={engine_event} source_declaration_ref={} prepared_mapping_verified=true playing_id={} object={:?} selected_source={} frames={} decoded_peak={} dry_peak={} authored_Play_actor=815416372 physical_audibility_not_claimed=true",
                        source.event,
                        source.playing_id,
                        source.object,
                        source.selected_source,
                        source.frames,
                        source.decoded_peak,
                        source.dry_peak
                    );
                    break;
                }
            }
        }
        let Some(event) = self.breath_start else {
            return Ok(());
        };
        if self.breath_voice.is_none() {
            if let Some(admission) = audio.admissions.iter().skip(self.audio_before).find(|row| {
                row.event == event && row.object == self.breath_object && row.error.is_none()
            }) {
                self.breath_voice = Some(admission.playing_id);
                eprintln!(
                    "ACTION_SMOKE_BLINK_WATER_SURFACE_PLAY event={event:#010x} playing_id={} object={} actual_admission=true",
                    admission.playing_id, admission.object
                );
            }
        }
        if let Some(id) = self.breath_voice {
            if let Some(source) = audio.source_samples.iter().find(|row| {
                row.playing_id == id
                    && row.event == event
                    && row.object == self.breath_emitter
                    && row.frames > 0
                    && row.dry_peak > 0.
            }) {
                self.breath_source = true;
                eprintln!(
                    "ACTION_SMOKE_PASS check=blink_water_surface_breath_actual_source event={event:#010x} playing_id={id} object={:?} source={:#010x} frames={} dry_peak={} reason={} physical_audibility_not_claimed=true",
                    self.breath_emitter,
                    source.selected_source,
                    source.frames,
                    source.dry_peak,
                    source.reason
                );
            }
        }
        Ok(())
    }
    pub(super) fn poll(&mut self, world: &mut World) -> Result<bool, String> {
        if let Some(input) = self.release.take() {
            send(world, input, false)?;
            return Ok(false);
        }
        let audio_result = self.mix(world).and_then(|()| self.surface_emitters(world));
        self.optional_audio(audio_result);
        let stamp = self.stamp(world)?;
        let p = player(world)?;
        let swimming = p.is_swimming();
        let blinking = p.machine.is_active(StateId::BLINKING);
        self.underwater_seen |= swimming && p.is_underwater();
        if self.released.is_some() {
            self.mana_spent |= p.mana() < self.mana_before;
            if blinking {
                self.blink_seen = true;
                self.blink_in_water |= swimming;
                if self.entered.is_none() {
                    self.entered = Some(stamp);
                    eprintln!(
                        "ACTION_SMOKE_BLINK_WATER_MOTION_ENTER stamp={stamp:?} feet={:?} swimming={swimming}",
                        p.feet()
                    );
                }
            }
            if self.last_water && !swimming && self.water_exit.is_none() {
                self.water_exit = Some(stamp);
                self.exit_during_blink = self.last_blink || blinking;
                eprintln!(
                    "ACTION_SMOKE_BLINK_WATER_MODE_EXIT stamp={stamp:?} feet={:?} prior_blink={} current_blink={blinking} states={:?}",
                    p.feet(),
                    self.last_blink,
                    p.active_player_states()
                );
            }
            if self.last_blink && !blinking && self.finished.is_none() {
                self.finished = Some(stamp);
                let mut report = String::new();
                p.write_combat_diagnostics(&mut report);
                eprintln!("ACTION_SMOKE_BLINK_WATER_MOTION_EXIT stamp={stamp:?}{report}");
            }
            if self.finished.is_some()
                && self.cooldown_ready.is_none()
                && p.selected_power().is_some_and(|cast| {
                    cast.settings.id == 2
                        && cast.last_cast_time().is_some()
                        && cast.cooldown_ready(stamp.clock, true)
                })
            {
                self.cooldown_ready = Some(stamp);
                eprintln!(
                    "ACTION_SMOKE_BLINK_WATER_COOLDOWN stamp={stamp:?} last_cast={:?} authored_duration={}",
                    p.selected_power().and_then(|cast| cast.last_cast_time()),
                    p.selected_power().unwrap().settings.cooldown
                );
            }
        }
        self.last_water = swimming;
        self.last_blink = blinking;
        if self.since.elapsed() > Duration::from_secs(90) {
            let mut report = String::new();
            p.write_combat_diagnostics(&mut report);
            if self.diving {
                send(world, self.down, false)?;
                self.diving = false;
            }
            if self.rising {
                send(world, self.up, false)?;
                self.rising = false;
            }
            if self.using {
                send(world, self.use_input, false)?;
                self.using = false;
            }
            return Err(format!(
                "water Blink input timeout stage={:?} underwater={} blink={} blink_in_water={} exit={:?} during_blink={} spent={} surface_voice={:?} source={} timings={:?}/{:?}/{:?}/{:?}/{:?}{report}",
                self.stage,
                self.underwater_seen,
                self.blink_seen,
                self.blink_in_water,
                self.water_exit,
                self.exit_during_blink,
                self.mana_spent,
                self.breath_voice,
                self.breath_source,
                self.released,
                self.entered,
                self.finished,
                self.cooldown_ready,
                self.rearm_input
            ));
        }
        match self.stage {
            Stage::Water if swimming => {
                //The fixture restore must finish its real mode transition
                //before the ordinary selection request is issued.
                send(world, self.selection, true)?;
                self.release = Some(self.selection);
                send(world, self.up, true)?;
                self.rising = true;
                self.stage(Stage::Select, world)?;
            }
            Stage::Select if p.selected_power_hud_status() == Some((2, true)) => {
                self.stage(Stage::Surface, world)?;
            }
            Stage::Surface if !p.is_underwater() => {
                send(world, self.up, false)?;
                self.rising = false;
                //The measured pair belongs to the forthcoming dive/cast,
                //not a setup-only surface recovery after the feet restore.
                self.stage(Stage::Orient, world)?;
            }
            Stage::Orient => {
                if Self::steer(world)? {
                    //Use last actually published surface values at the dive
                    //boundary, rather than an earlier setup fade-in-flight.
                    self.mix_baseline = None;
                    self.mix_underwater = None;
                    self.mix_surface = false;
                    self.mix_value = None;
                    let audio_result = self.mix(world);
                    self.optional_audio(audio_result);
                    send(world, self.down, true)?;
                    self.diving = true;
                    self.stage(Stage::Dive, world)?;
                }
            }
            Stage::Dive if p.is_underwater() && p.breath() < 100. => {
                //One finite physical dive. Aiming must never keep lowering
                //the player merely because the selected target is unsuitable.
                send(world, self.down, false)?;
                self.diving = false;
                // Suppressed while SWIM: only a later actual surface Start
                // can qualify, never a pre-dive admission of the same event.
                self.audio_before = super::super::audio::observation::snapshot(world)
                    .admissions
                    .len();
                send(world, self.use_input, true)?;
                self.using = true;
                self.stage(Stage::Aim, world)?;
            }
            Stage::Aim => {
                // Follow the same fixed real land landmark as the eye moves
                // during the physical dive. The native target must still pass.
                //Steering convergence only belongs to initial setup. While
                //diving the eye moves: it must not gate a valid native cast.
                let _ = Self::steer(world)?;
                let p = player(world)?;
                if !p.machine.is_active(StateId::USE_POWER)
                    || !p
                        .selected_power()
                        .is_some_and(|cast| cast.settings.id == 2 && cast.phase() == Phase::Aiming)
                {
                    return Ok(false);
                }
                let owner = world
                    .get_resource::<super::super::player_actions::PlayerActions>()
                    .and_then(|actions| actions.blink.as_ref());
                let Some(owner) = owner else { return Ok(false) };
                let Some(target) = owner.blink.target.as_ref() else {
                    return Ok(false);
                };
                let plane = p
                    .swimming_contact
                    .ok_or("water Blink lost its actual water sample while aiming")?
                    .plane;
                if !self.aim_observed {
                    self.aim_observed = true;
                    eprintln!(
                        "ACTION_SMOKE_BLINK_WATER_AIM_OBSERVED stamp={stamp:?} native_target={target:?} initial_cast_obstruction={:?} sampled_plane={plane} underwater={} feet={:?} camera={:?} steering_precision_is_not_cast_gate=true",
                        owner.blink.obstruction,
                        p.is_underwater(),
                        p.feet(),
                        p.published_camera_frame()
                    );
                }
                //61ec20 has no head-depth veto. The handoff claim concerns
                //actual SWIM mode; log actual head depth rather than forcing it.
                if target.position.z <= plane || !p.is_swimming() {
                    return Ok(false);
                }
                eprintln!(
                    "ACTION_SMOKE_BLINK_WATER_ACTUAL_TARGET target={target:?} sampled_plane={plane} feet={:?} camera={:?} states={:?} no_target_override=true",
                    p.feet(),
                    p.published_camera_frame(),
                    p.active_player_states()
                );
                self.cast_start = p.feet();
                self.mana_before = p.mana();
                self.released = Some(stamp);
                send(world, self.use_input, false)?;
                self.using = false;
                self.stage(Stage::Flight, world)?;
            }
            Stage::Flight if self.blink_seen && !blinking && self.mana_spent => {
                self.stage(Stage::Recovery, world)?;
            }
            Stage::Recovery
                if !swimming
                    && p.physics.near_ground()
                    && !p.power_cast_active(2)
                    && self.cooldown_ready.is_some() =>
            {
                if !self.blink_in_water || !self.exit_during_blink || p.feet() == self.cast_start {
                    return Err("water Blink did not demonstrate actual underwater12 to out-of-water mode handoff".into());
                }
                self.rearm_input = Some(stamp);
                send(world, self.use_input, true)?;
                self.using = true;
                self.stage(Stage::Rearm, world)?;
            }
            Stage::Rearm
                if p.machine.is_active(StateId::USE_POWER)
                    && p.selected_power()
                        .is_some_and(|cast| cast.phase() == Phase::Aiming) =>
            {
                eprintln!(
                    "ACTION_SMOKE_PASS check=blink_water_actual_handoff_and_rearm release={:?} entered={:?} finished={:?} water_exit={:?} cooldown={:?} rearm_input={:?} rearm_active={stamp:?} feet={:?} release_to_finish_wall_clock_frames={:?} finish_to_cooldown_wall_clock_frames={:?} rearm_input_to_active_wall_clock_frames={:?} no_timing_threshold_or_clock_override=true",
                    self.released,
                    self.entered,
                    self.finished,
                    self.water_exit,
                    self.cooldown_ready,
                    self.rearm_input,
                    p.feet(),
                    self.finished
                        .zip(self.released)
                        .map(|(end, start)| end.elapsed(start)),
                    self.cooldown_ready
                        .zip(self.finished)
                        .map(|(end, start)| end.elapsed(start)),
                    self.rearm_input.map(|start| stamp.elapsed(start))
                );
                send(world, self.cancel, true)?;
                self.release = Some(self.cancel);
                self.stage(Stage::Cancel, world)?;
            }
            Stage::Cancel if !p.power_cast_active(2) => {
                if self.using {
                    send(world, self.use_input, false)?;
                    self.using = false;
                }
                self.stage(Stage::Done, world)?;
            }
            Stage::Done => {
                let audio_result = self.audio(world);
                self.optional_audio(audio_result);
                eprintln!(
                    "ACTION_SMOKE_PASS check=blink_water_sequence_complete actual_mode_and_motion=true exit=ordinary_ExitDrain no_pixels_or_heard_mix_claim=true"
                );
                eprintln!(
                    "ACTION_SMOKE_BLINK_WATER_OPTIONAL_AUDIO mix_baseline={:?} mix_underwater={:?} mix_surface={} authored_surfacing_fx_source={} actual_heavy_breath_play={} actual_heavy_breath_source={} observation_error={:?} unavailable_receipts_are_not_ACKs=true",
                    self.mix_baseline,
                    self.mix_underwater,
                    self.mix_surface,
                    self.surface_fx_source,
                    self.breath_voice.is_some(),
                    self.breath_source,
                    self.audio_error
                );
                return Ok(true);
            }
            _ => {}
        }
        let audio_result = self.audio(world);
        self.optional_audio(audio_result);
        Ok(false)
    }
    fn optional_audio(&mut self, result: Result<(), String>) {
        if let Err(error) = result {
            if self.audio_error.is_none() {
                eprintln!(
                    "ACTION_SMOKE_BLINK_WATER_AUDIO_UNAVAILABLE error={error} not_a_motion_gate=true"
                );
                self.audio_error = Some(error);
            }
        }
    }
}
