//! Accepted FX sound commands shared with the existing main audio owner.
use bevy::{prelude::*, render::extract_resource::ExtractResource};
use std::{collections::HashMap, sync::{Arc, Mutex, MutexGuard}};
use crate::{gameplay::{effects::sound::handle::{SoundHandle, SoundObjectWorld}, sound::{
    acoustic_world::{AcousticWorld, AdmissionListener}, admission::{self, ListenerMotion},
    objects::EffectObjects, playing::{Event, Notification, NotificationKind, PlayingEvents, PlayingId, Ticket}, Emitter,
}}, rendering::particles::world_sources::WorldSoundEvent};

#[derive(Clone, Copy)]
pub(crate) struct SoundFrame {
    pub axis: Mat3,
    pub scale: f32,
    ///22fd00 receives the actual FX contact speed squared times mass.
    pub energy:f32,
    /// Actual60a8b0 callback owner, retained by the accepted sound cookie.
    pub entity_context: Option<super::world_effects::EntityEffectContext>,
}

pub(crate) struct Stimulus {
    pub owner: crate::gameplay::entity_lifetime::EntityId,
    pub object: Emitter,
    pub position: Vec3,
    pub distance: f32,
    pub descriptor: Arc<crate::assets::wwise::game_properties::GameSoundProperties>,
}
struct StimulusCookie {
    owner: crate::gameplay::entity_lifetime::EntityId,
    descriptor: Arc<crate::assets::wwise::game_properties::GameSoundProperties>,
    distance: f32,
}

pub(crate) struct SoundObject {
    pub position: Vec3,
    pub front: Vec3,
    pub pending_position: Vec3,
    pub pending_front: Vec3,
    pub displacement: Vec3,
    pub scale: f32,
    pub energy: Option<(u32, f32)>,
    /// Sound object +0xa8 (1400c3b70): the source zone's occlusion weight in
    /// path scoring. The +0xa4 level scale (1400c3b40) stays at its default.
    pub source_occlusion: bool,
    pub audible_mask: u32,
    pub range: f32,
    events: Vec<(PlayingId, f32)>,
    retiring: bool,
}
impl SoundObject {
    pub fn has_events(&self) -> bool { !self.events.is_empty() }
    pub fn publish_frame(&mut self, frozen: bool) {
        // c68e0 computes displacement before its pause/empty-object gates.
        self.displacement = self.pending_position - self.position;
        if !frozen && self.has_events() {
            self.position = self.pending_position;
            self.front = self.pending_front;
        }
    }
    fn finished(&mut self, id: PlayingId) {
        if let Some(index) = self.events.iter().position(|&(playing, _)| playing == id) {
            let (_, range) = self.events.swap_remove(index);
            // c47a0 recomputes only when the removed positive-range cookie
            // supplied the cached maximum. No positive cookies leaves -1.
            if range > 0.0 && range == self.range {
                self.range = self.events.iter().map(|&(_, range)| range)
                    .filter(|&range| range > 0.0).fold(-1.0, f32::max);
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct SoundState {
    pub events: HashMap<String, Result<u32, String>>,
    pub declarations: Vec<Option<Arc<WorldSoundEvent>>>,
    pub objects: HashMap<Emitter, SoundObject>,
    pub queued: Vec<Ticket>,
    pub released: Vec<Emitter>,
    pub playing: PlayingEvents,
    pub(crate) playing_ids: Option<crate::gameplay::sound::playing::PlayingIds>,
    pub notifications: Vec<Notification>,
    stimulus_cookies: HashMap<PlayingId, StimulusCookie>,
    pub stimuli: std::collections::VecDeque<Stimulus>,
    pub acoustics: Option<Arc<Mutex<AcousticWorld>>>,
    pub listeners: [Option<AdmissionListener>; 8],
    pub motion: [Option<ListenerMotion>; 8],
    pub identities: Option<EffectObjects>,
    pub energy: Option<crate::assets::wwise::RtpcDeclaration>,
    pub ready: bool,
    pub ready_sources: u64,
}
impl SoundState {
    pub(crate) fn references_stimulus_owner(&self,owner:crate::gameplay::entity_lifetime::EntityId)->bool{
        self.stimulus_cookies.values().any(|cookie|cookie.owner==owner)
            || self.stimuli.iter().any(|packet|packet.owner==owner)
    }
    pub fn pump_notifications(&mut self) {
        let notifications = self.playing.take_notifications();
        for notification in &notifications {
            // c7180 promotes the first Duration packet to8000; later8 packets
            //still reach FX bookkeeping. c47a0 supplies8000 beforeEnd when
            //none arrived. Only the actual stimulus cookie is consumed once.
            if matches!(notification.kind, NotificationKind::SourceReady | NotificationKind::EventPlaying) {
                if let Some(cookie)=self.stimulus_cookies.remove(&notification.id) {
                    if let Some(object)=self.objects.get(&notification.event.object) {
                        self.stimuli.push_back(Stimulus { owner:cookie.owner, object:notification.event.object,
                            position:object.position, distance:cookie.distance, descriptor:cookie.descriptor });
                    }
                }
            }
            if matches!(notification.kind, NotificationKind::SourceReady) {
                if self.ready_sources == 0 {
                    info!(declaration = notification.event.declaration, "World FX audio source readiness confirmed by shared output");
                }
                self.ready_sources += 1;
            }
            if matches!(notification.kind, NotificationKind::End) {
                if let Some(object) = self.objects.get_mut(&notification.event.object) {
                    object.finished(notification.id);
                }
            }
        }
        self.notifications.extend(notifications);
    }
}

#[derive(Resource, Clone, Default, ExtractResource)]
pub(crate) struct WorldSounds(Arc<Mutex<SoundState>>);
impl WorldSounds {
    pub fn lock(&self) -> MutexGuard<'_, SoundState> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl SoundObjectWorld for WorldSounds {
    type Object = Emitter;
    type Playing = PlayingId;
    type Context = SoundFrame;
    fn available(&self) -> bool { self.lock().ready }
    fn allocate(&mut self, position: Vec3, context: &SoundFrame) -> Result<Option<Emitter>, String> {
        let mut state = self.lock();
        let emitter = state.identities.as_ref().ok_or("FX sound identities are unavailable")?.allocate()?;
        state.objects.insert(emitter, SoundObject {
            position, pending_position: position, front: context.axis.x_axis,
            pending_front: context.axis.x_axis, displacement: Vec3::ZERO,
            scale: 1.0, energy: None, source_occlusion: true, audible_mask: 0,
            range: -1.0, events: Vec::new(), retiring: false,
        });
        Ok(Some(emitter))
    }
    fn post(&mut self, _: &mut SoundHandle<Emitter, PlayingId>, object: Emitter,
        event: u32, context: &SoundFrame) -> Result<Option<PlayingId>, String>
    {
        let mut state = self.lock();
        if state.objects.get(&object).is_some_and(|object| object.retiring) { return Err("FX sound object is retiring".into()); }
        let declaration = state.declarations.get(event as usize).and_then(Option::as_ref)
            .ok_or("FX sound declaration reference is absent")?.clone();
        let declaration = &declaration.declaration;
        let source = state.objects.get(&object).ok_or("FX sound object is absent")?;
        // c4550's short-event query is not the portal query used for replay.
        if declaration.game_properties.is_none() && !declaration.game_defined
            && declaration.max_distance > 0.0 && !declaration.persistent_loop()
            && declaration.duration[1] <= 2.0
            && admission::motion_inaudible(source.position, source.displacement, declaration.max_distance, &state.motion)
        { return Ok(None); }
        if !state.playing.can_register_object(object) { return Err("sound object cookie pool is full".into()); }
        let playing_id = state.playing_ids.as_ref().ok_or("shared audio playing-ID owner is absent")?.allocate();
        let ticket = state.playing.register_with_id(playing_id, Event { object, declaration: event, callbacks: 0x8009 })?;
        let id = ticket.id();
        if let Some(context)=context.entity_context.filter(|context|
            context.contact_stimulus_enabled==Some(true) && context.contact_speed>=0.) {
            if let (Some(owner),Some(descriptor))=(context.owner,declaration.game_descriptor.as_ref()) {
                state.stimulus_cookies.insert(id,StimulusCookie{owner,descriptor:descriptor.clone(),
                    distance:if declaration.game_defined {descriptor.propagation_distance} else {declaration.max_distance}});
            }
        }
        let source = state.objects.get_mut(&object).ok_or("FX sound object was removed")?;
        source.events.push((id, declaration.max_distance));
        if source.range < declaration.max_distance && declaration.max_distance > 0.0 {
            source.range = declaration.max_distance;
        }
        if declaration.game_defined {
            // c4550's authored game-defined event has no middleware action:
            //the canonical cookie emits8000 synchronously, thenEnd.
            ticket.source_ready();
        } else { state.queued.push(ticket); }
        Ok(Some(id))
    }
    fn release(&mut self, object: Emitter, stop: bool) -> Result<(), String> {
        let mut state = self.lock();
        if !state.objects.contains_key(&object) || state.released.contains(&object) {
            return Err("FX sound object released twice".into());
        }
        if stop {
            // Accepted tickets retain their object until submission and the
            // ordered return marker; the audio consumer owns final removal.
            state.objects.get_mut(&object).expect("validated sound object").retiring = true;
            state.released.push(object);
        } else { state.objects.remove(&object); }
        Ok(())
    }
    fn initialize_posted(&mut self, object: Emitter, context: &SoundFrame) -> Result<(), String> {
        let mut state = self.lock();
        let energy = state.energy.ok_or("FX physics-energy RTPC is unavailable")?;
        let object = state.objects.get_mut(&object).ok_or("FX sound object is absent")?;
        // Ordinary world NodeFx21dc10 supplies zero; real ContactFX retains
        //its60a480 speed/mass rather than inheriting that unrelated default.
        object.energy = Some((energy.id, energy.clamp(context.energy)));
        object.source_occlusion = false; // 22fac0 ->c3b70(0), after successful post.
        Ok(())
    }
    fn object_inaudible(&mut self, object: Emitter) -> Result<bool, String> {
        self.lock().objects.get(&object).map(|object| object.audible_mask == 0)
            .ok_or_else(|| "FX sound object is absent".into())
    }
    fn object_range(&self, object: Emitter) -> Result<f32, String> {
        self.lock().objects.get(&object).map(|object| object.range)
            .ok_or_else(|| "FX sound object is absent".into())
    }
    fn point_inaudible(&mut self, position: Vec3, range: f32) -> Result<bool, String> {
        let state = self.lock();
        let acoustics = state.acoustics.as_ref().ok_or("FX acoustic world is unavailable")?.clone();
        let listeners = state.listeners;
        drop(state);
        let result = acoustics.lock().unwrap_or_else(|error| error.into_inner())
            .point_inaudible(position, range, true, &listeners);
        result
    }
    fn update_position(&mut self, object: Emitter, position: Vec3, context: &SoundFrame) -> Result<(), String> {
        let mut state = self.lock();
        let object = state.objects.get_mut(&object).ok_or("FX sound object is absent")?;
        // c5b70 writes the pending frame, not current position/displacement.
        object.pending_position = position;
        object.pending_front = context.axis.x_axis;
        object.scale = context.scale;
        Ok(())
    }
}
