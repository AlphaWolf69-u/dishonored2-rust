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
const TRAVEL: &str = "models/effects/gameplay/powers/blink/blink_travel_01.fx";
const CUBE: &str = "env/tonemapping/colorcube";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage { Select, AimCancel, Cancel, Rearm, AimCast, Cast, Recharge, Holster, Redraw, Done }

#[derive(Clone, Copy)]
struct FirstEnvironment {
    order: u64,
    age: f32,
    duration_end: f32,
    native_zero: bool,
}
struct BlinkAudioWindow {
    epoch:u64,closed:bool,world_source_seen:bool,
    world_voices:usize,paused_world_voices:usize,master_gain_min:Option<f32>,
    all_world_paused_seen:bool,unpaused_world_seen:bool,
}
impl BlinkAudioWindow {
    fn observe(&mut self,audio:&app::audio::observation::Snapshot,active:bool){
        if active {
            self.world_voices=self.world_voices.max(audio.output.world_component_voices);
            self.paused_world_voices=self.paused_world_voices.max(audio.output.paused_world_component_voices);
            self.all_world_paused_seen|=audio.output.world_component_voices>0&&audio.output.world_component_voices==audio.output.paused_world_component_voices;
            self.unpaused_world_seen|=audio.output.world_component_voices>audio.output.paused_world_component_voices;
            if let Some(gain)=audio.output.master_target_gain {self.master_gain_min=Some(self.master_gain_min.map_or(gain,|old|old.min(gain)));}
        }
        if !self.world_source_seen {
            if let Some(source)=audio.source_samples.iter().find(|source|source.interval==self.epoch
                &&matches!(source.object,crate::gameplay::sound::Emitter::Component{..}|crate::gameplay::sound::Emitter::ComponentJoint{..})
                &&source.decoded_peak>0.&&(source.dry_peak>0.||source.aux_peak>0.)) {
                self.world_source_seen=true;
                eprintln!("ACTION_SMOKE_BLINK_WORLD_SOURCE interval={} playing_id={} object={:?} source={:#010x} interval_frames={} decoded_peak={} dry_peak={} aux_peak={} lifetime_peaks_not_used=true stage=post_voice_pre_bus physical_audibility_not_claimed=true",
                    self.epoch,source.playing_id,source.object,source.selected_source,source.frames,source.decoded_peak,source.dry_peak,source.aux_peak);
            }
        }
    }
}

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
    travel_owner: Option<(String, u64)>,
    travel_environments: Vec<std::sync::Weak<crate::rendering::environment_resources::PreparedEnvironment>>,
    travel_emitters: Vec<crate::gameplay::sound::Emitter>,
    travel_started: bool,
    travel_environment_seen: bool,
    travel_environment_zero_seen: bool,
    travel_environment_first: Option<FirstEnvironment>,
    travel_source_seen: bool,
    travel_environment_trace: Option<String>,
    audio_window:Option<BlinkAudioWindow>,
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
            regen_source_ready: false, regen_callback: false, audio_before,
            travel_owner: None, travel_environments: Vec::new(), travel_emitters: Vec::new(),
            travel_started: false, travel_environment_seen: false, travel_source_seen: false,
            travel_environment_zero_seen: false, travel_environment_first: None,
            travel_environment_trace: None, audio_window:None })
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

    /// Retain only real owner/emitter identities and weak metadata identities.
    /// No FX request, admission, clock, image readiness or source is fabricated.
    fn observe_travel(&mut self, world: &World) -> Result<(), String> {
        use std::sync::{Arc, atomic::Ordering};
        if self.travel_owner.is_none() {
            if let Some(entry) = world.get_resource::<crate::rendering::bevy_backend::world_effects::DynamicEffects>()
                .and_then(|effects| effects.0.iter().find(|entry| entry.effect == TRAVEL)) {
                let definition = entry.sources.effects.get(TRAVEL)
                    .ok_or("actual Blink travel owner has no acquired definition")?
                    .as_ref().map_err(Clone::clone)?;
                for group in definition.groups.iter().filter(|group| group.asset.enabled) {
                    for source in &group.environments {
                        if !group.asset.environments[source.index].enabled
                            || !source.environment.values.contains_key(CUBE) { continue; }
                        if self.travel_environments.len() == crate::rendering::environment_effect_runtime::ENVIRONMENT_CAPACITY {
                            return Err("Blink travel metadata exceeds the actual64 environment family slots".into());
                        }
                        self.travel_environments.push(Arc::downgrade(&source.environment));
                    }
                }
                if self.travel_environments.is_empty() {
                    return Err("actual Blink travel definition has no enabled colour-cube environment".into());
                }
                self.travel_owner = Some((entry.owner.clone(), entry.revision));
                eprintln!("ACTION_SMOKE_BLINK_TRAVEL_OWNER owner={} revision={} effect={} environment_identities={}",
                    entry.owner, entry.revision, entry.effect, self.travel_environments.len());
            }
        }
        let Some((owner, revision)) = &self.travel_owner else { return Ok(()) };
        let Some(simulation) = world.get_resource::<crate::rendering::bevy_backend::camera_scene::FxSimulation>() else { return Ok(()) };
        let started = simulation.observe_fx_owner(owner);
        self.travel_started |= !started.handles.is_empty() && started.pending_starts == 0 && started.environments > 0;
        // Preserve the first actual eligible producer state, rather than
        // accepting any later zero after an otherwise positive envelope.
        let mut eligible = 0;
        let mut candidate = None;
        if !self.travel_source_seen {
            for emitter in simulation.observe_fx_owner_audio(owner)? {
                if self.travel_emitters.contains(&emitter) { continue; }
                //19a7e0/19fcc0:512 real sound-family slots; one actual cast.
                if self.travel_emitters.len() == 512 {
                    return Err("Blink travel observation exceeds actual512 sound-family identities".into());
                }
                self.travel_emitters.push(emitter);
            }
        }
        if !self.travel_environment_seen && !self.travel_environment_zero_seen {
            use std::fmt::Write as _;
            let mut trace = String::new();
            let _ = write!(trace,"owner={owner} revision={revision} pending={} environment_heads={} ",
                started.pending_starts, started.environments);
            simulation.observe_fx_owner_environments(owner,|age,effect|{
                let timing=&effect.timing;
                if effect.view_mask & 1 != 0 {
                    eligible += 1;
                    candidate=Some(FirstEnvironment {
                        order: effect.registration_order, age, duration_end: timing.duration_end,
                        //21a240 initializes last_main from fadein(0);21a090
                        //jumps to fadeout*last_main at duration_end.
                        native_zero: effect.active && !effect.can_loop()
                            && age >= timing.duration_end && timing.duration_end > timing.fade_in_end
                            && timing.fade_in_end == timing.delay_end
                            && timing.fade_out_rate.is_finite() && timing.fade_out_rate > 0.
                            && timing.fading_out && effect.initial_main == 1.
                            && effect.last_main == 0. && effect.blend == 0.,
                    });
                }
                let _=write!(trace,"producer[order={} age={} active={} mask={:#x} blend={} initial_main={} last_main={} delay_end={} fadein_end={} duration_end={} fadein_rate={} duration_rate={} fadeout_rate={} fading_out={}] ",
                    effect.registration_order,age,effect.active,effect.view_mask,effect.blend,effect.initial_main,effect.last_main,
                    timing.delay_end,timing.fade_in_end,timing.duration_end,timing.fade_in_rate,timing.duration_rate,timing.fade_out_rate,timing.fading_out);
            })?;
            if self.travel_environment_first.is_none() {
                if let Some(mut first)=candidate {
                    first.native_zero &= eligible == 1 && self.travel_environments.len() == 1
                        && started.environments == 1;
                    self.travel_environment_first=Some(first);
                }
            }
            if let Some(images)=world.get_resource::<crate::rendering::bevy_backend::environment_images::EnvironmentImages>(){
                let _=write!(trace,"image_generation={} image_ACK={} ",images.generation,images.completed.load(Ordering::Acquire));
            }
            if let Some(effects)=world.get_resource::<app::environment_effects::EnvironmentEffects>(){
                let _=write!(trace,"frame_revision={} frame_rows=[",effects.revision);
                for row in effects.frame.rows(){
                    let _=write!(trace,"({},{:p},{},{})",row.environment.name,Arc::as_ptr(&row.environment),row.weight,row.influence);
                }
                let _=write!(trace,"] expected=[");
                for source in &self.travel_environments{
                    if let Some(environment)=source.upgrade(){
                        let _=write!(trace,"({},{:p},prepared={},image={:?})",environment.name,Arc::as_ptr(&environment),
                            environment.prepared.load(Ordering::Acquire),effects.image(&environment,CUBE).map(|image|image.id()));
                    }else{let _=write!(trace,"(retired_identity)");}
                }
                let _=write!(trace,"] ");
            }else{let _=write!(trace,"frame_owner_absent=true ");}
            if let Some(display)=world.get_resource::<crate::rendering::bevy_backend::display_scene::DisplaySceneFrame>()
                .and_then(|frame|frame.0.as_ref()){
                let _=write!(trace,"display_camera={:?} ",display.camera);
                if let Some(overrides)=&display.overrides{
                    let _=write!(trace,"display_images={:?} display_weights={:?}",overrides.images.each_ref().map(Handle::id),overrides.weights);
                }else{let _=write!(trace,"display_overrides_absent=true");}
            }else{let _=write!(trace,"display_frame_absent=true");}
            if self.travel_environment_trace.as_ref()!=Some(&trace){
                eprintln!("ACTION_SMOKE_BLINK_ENV_GATE FX_frame={:?} {trace}",started.frame);
                self.travel_environment_trace=Some(trace);
            }
        }
        if !self.travel_started || self.travel_environment_seen || self.travel_environment_zero_seen { return Ok(()) }
        let Some(effects) = world.get_resource::<app::environment_effects::EnvironmentEffects>() else { return Ok(()) };
        let Some(display) = world.get_resource::<crate::rendering::bevy_backend::display_scene::DisplaySceneFrame>()
            .and_then(|frame| frame.0.as_ref()) else { return Ok(()) };
        // This separate receipt describes the observed native zero-output
        // edge. It never stands in for a positive contribution or GPU pixels.
        if let (Some(first), Some(current)) = (self.travel_environment_first, candidate) {
            let image_ack = world.get_resource::<crate::rendering::bevy_backend::environment_images::EnvironmentImages>()
                .is_some_and(|images| images.generation != 0 && images.completed.load(Ordering::Acquire) == images.generation);
            if first.native_zero && current.native_zero && first.order == current.order
                && eligible == 1 && started.environments == 1 && started.pending_starts == 0
                && self.travel_environments.len() == 1 && effects.frame.rows().len() == 1
                && display.overrides.is_none() && image_ack {
                if let Some(environment) = self.travel_environments[0].upgrade() {
                    let row = &effects.frame.rows()[0];
                    if Arc::ptr_eq(&row.environment,&environment) && row.weight == 0. && row.influence == 1.
                        && environment.prepared.load(Ordering::Acquire) {
                        let image = effects.image(&environment,CUBE)?;
                        self.travel_environment_zero_seen=true;
                        eprintln!("ACTION_SMOKE_PASS check=blink_travel_environment_native_zero_weight owner={owner} revision={revision} order={} first_observed_eligible_age={} duration_end={} first_last_main_zero=true first_blend_zero=true initial_main=1 zero_fadein=true source_branch=21a240_21a090 current_age={} environment={} identity={:p} source_image={:?} GPU_image_prepared=true image_generation_ACK=true Frame_weight=0 Frame_influence=1 display_overrides_absent=true native_zero_output_matched=true positive_environment_contribution=false pixels_verified=false",
                            first.order,first.age,first.duration_end,current.age,environment.name,Arc::as_ptr(&environment),image.id());
                        return Ok(());
                    }
                }
            }
        }
        let Some(display) = display.overrides.as_ref() else { return Ok(()) };
        for source in &self.travel_environments {
            let Some(environment) = source.upgrade() else { continue };
            let Some(row) = effects.frame.rows().iter().find(|row| Arc::ptr_eq(&row.environment, &environment)
                && row.weight > 0. && row.influence > 0.) else { continue };
            if !environment.prepared.load(Ordering::Acquire) { continue; }
            let image = effects.image(&environment, CUBE)?;
            let weights = display.weights.to_array();
            let Some(index) = display.images.iter().enumerate().position(|(index, candidate)|
                candidate.id() == image.id() && weights[index] > 0.) else { continue };
            self.travel_environment_seen = true;
            eprintln!("ACTION_SMOKE_PASS check=blink_travel_actual_environment_display owner={owner} revision={revision} environment={} identity={:p} GPU_image_prepared=true native_environment_started=true row_weight={} row_influence={} display_image={:?} display_slot={index} display_weight={} display_publication=true pixels_verified=false",
                environment.name, Arc::as_ptr(&environment), row.weight, row.influence, image.id(), weights[index]);
            break;
        }
        Ok(())
    }
    fn observe_travel_audio(&mut self, world: &World, audio: &app::audio::observation::Snapshot) -> Result<(), String> {
        if self.travel_source_seen { return Ok(()) }
        let Some(sounds) = world.get_resource::<crate::rendering::bevy_backend::world_sounds::WorldSounds>() else { return Ok(()) };
        let sounds = sounds.lock();
        for source in &audio.source_samples {
            if !self.travel_emitters.contains(&source.object) { continue; }
            // WorldSounds::post stores a prepared declaration-reference index
            // in the native Ticket. Engine::submit records declaration.id.
            // Meter preserves the Ticket domain; never compare those IDs as
            // though an FX reference were an authored middleware event ID.
            let engine_event = sounds.declarations.get(source.event as usize)
                .and_then(Option::as_ref)
                .ok_or("actual Blink source declaration reference is no longer owned")?
                .declaration.id;
            let object = format!("{:?}", source.object);
            let Some(admission) = audio.admissions.iter().skip(self.audio_before).find(|admission|
                admission.playing_id == source.playing_id && admission.object == object
                    && admission.event == engine_event) else { continue };
            if let Some(error) = &admission.error { return Err(format!("actual Blink travel sound admission failed: {error}")); }
            if !audio.source_processed_ids.contains(&source.playing_id) || source.frames == 0
                || !(source.decoded_peak > 0.) || !(source.dry_peak > 0. || source.aux_peak > 0.) { continue; }
            self.travel_source_seen = true;
            eprintln!("ACTION_SMOKE_PASS check=blink_travel_actual_sound_nonzero owner={:?} source_declaration_ref={} admitted_engine_event={:#010x} prepared_mapping_verified=true playing_id={} object={:?} selected_source={:#010x} actual_admission=true source_processed=true frames={} decoded_peak={} post_voice_dry_peak={} post_voice_aux_peak={} speaker_gain_peak={} route_bus={:?} route_target_peak={:?} master_target_peak={:?} reason={} final_bus_contribution_or_physical_audibility_not_claimed=true",
                self.travel_owner, source.event, engine_event, source.playing_id, source.object, source.selected_source, source.frames,
                source.decoded_peak, source.dry_peak, source.aux_peak, source.gain_peak, source.route_bus,
                source.route_target_peak, source.master_target_peak, source.reason);
            break;
        }
        Ok(())
    }

    pub(super) fn poll(&mut self, world: &mut World) -> Result<bool, String> {
        let p = player(world)?;
        let blink_active=p.machine.is_active(StateId::BLINKING);
        self.cancel_seen |= p.machine.is_active(StateId::CANCEL_ACTION);
        self.blink_seen |= p.machine.is_active(StateId::BLINKING);
        self.mantle_seen |= p.machine.is_active(StateId::MANTLE);
        if matches!(self.stage, Stage::Cast | Stage::Recharge) {
            self.spent |= p.mana() < self.mana_before;
            self.mana_low = self.mana_low.min(p.mana());
        }
        if blink_active && self.audio_window.is_none(){
            let epoch=app::audio::observation::begin_blink_window(world)?;
            self.audio_window=Some(BlinkAudioWindow{epoch,closed:false,world_source_seen:false,world_voices:0,paused_world_voices:0,master_gain_min:None,all_world_paused_seen:false,unpaused_world_seen:false});
        }
        if !blink_active {
            if let Some(window)=self.audio_window.as_mut().filter(|window|!window.closed){
                app::audio::observation::end_blink_window(world,window.epoch);window.closed=true;
            }
        }
        if matches!(self.stage, Stage::Cast | Stage::Recharge) {
            self.observe_travel(world)?;
            let audio = app::audio::observation::snapshot(world);
            if let Some(window)=&mut self.audio_window {window.observe(&audio,blink_active);}
            self.observe_travel_audio(world, &audio)?;
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
        }
        if let Some(input) = self.release.take() { send(world, input, false)?; return Ok(false); }
        if self.since.elapsed() > Duration::from_secs(90) {
            let mut snapshot = String::new(); player(world)?.write_combat_diagnostics(&mut snapshot);
            return Err(format!("Blink actual-input timeout stage={:?} cancel74={} blink12={} spent={} travel_owner={:?} travel_started={} environment_display={} environment_native_zero={} sound_nonzero={} emitters={:?} snapshot={snapshot}", self.stage, self.cancel_seen, self.blink_seen, self.spent, self.travel_owner, self.travel_started, self.travel_environment_seen, self.travel_environment_zero_seen, self.travel_source_seen, self.travel_emitters));
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
                if player(world)?.mana() > self.mana_low && (self.regen_event.is_none() || self.regen_admitted)
                    && (self.travel_environment_seen || self.travel_environment_zero_seen) && self.travel_source_seen {
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
                if let Some(window)=&self.audio_window {
                    let output=app::audio::observation::blink_output(world,window.epoch);
                    eprintln!("ACTION_SMOKE_BLINK_SCENE_AUDIO interval={} actual_state12_window_closed={} world_component_source_nonzero={} final_master_interval={output:?} observed_world_voices_max={} observed_paused_world_voices_max={} all_world_components_paused_at_an_observation={} unpaused_world_components_observed={} observed_master_target_min={:?} aggregate_includes_real_travel_and_ambient_not_individual_bus_attribution=true physical_audibility_not_claimed=true",
                        window.epoch,window.closed,window.world_source_seen,window.world_voices,window.paused_world_voices,window.all_world_paused_seen,window.unpaused_world_seen,window.master_gain_min);
                    if !window.world_source_seen {eprintln!("ACTION_SMOKE_NOT_COVERED check=blink_environmental_source_survival reason=no_nonzero_world_component_source_in_actual_state12_window actual_pause_counts_reported=true no_substitute_sound_injected=true");}
                }
                if !(self.travel_environment_seen || self.travel_environment_zero_seen) || !self.travel_source_seen {
                    return Err("Blink cannot complete without actual positive ENV display or source-proven native zero-weight publication, plus nonzero sound source receipts".into());
                }
                eprintln!("ACTION_SMOKE_BLINK_ENV_RESULT positive_display={} source_proven_native_zero={} positive_pixels_not_claimed=true",self.travel_environment_seen,self.travel_environment_zero_seen);
                eprintln!("ACTION_SMOKE_PASS check=blink_input_sequence_complete GPU_record_receipt_required_separately=ACTION_SMOKE_FSDISTO_RENDER pixels_verified=false exit=ordinary_ExitDrain");
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
}
