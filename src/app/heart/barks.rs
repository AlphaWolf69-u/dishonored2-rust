//! AIBarks_Device5dbfb0/5dd450 and Speech4e6b80 on the actual Heart.
//! Only selected lines request Wwise media. Shared CoordinationBB owns the
//! cooldown records; immutable rule/Speech resources retain canonical leases.
use super::*;
use crate::{
    assets::{slash_rules::barks::Outcome, speech::Recording},
    gameplay::{
        ai_world::{Slot, Worlds},
        sound::bark_history::BarkRecord,
    },
};
use heart::barks::{self, Context, Selection};

pub(super) struct Device {
    definition: Arc<barks::Definition>,
    handlers: Vec<Vec<barks::Runtime>>,
    pending: Option<Pending>,
    voice: Option<Voice>,
    retiring: bool,
    completion_callbacks: usize,
}
struct Pending {
    selection: Selection,
    recording: Recording,
    remaining: f32,
}
struct Voice {
    recording: Recording,
    line: usize,
    event: Option<u32>,
    stopping: bool,
    priority: i32,
}
impl Device {
    pub(super) fn write_diagnostics(&self, report: &mut String) {
        use std::fmt::Write;
        let _ = writeln!(report, "      bark retiring={} pending_delay={:?} voice={:?} completion_callbacks={}", self.retiring, self.pending.as_ref().map(|pending| pending.remaining), self.voice.as_ref().map(|voice| (voice.line, voice.event, voice.stopping, voice.priority)), self.completion_callbacks);
    }
    pub(super) fn new(definition: Arc<barks::Definition>) -> Self {
        let handlers = definition
            .handlers
            .iter()
            .map(|(_, list)| list.iter().cloned().map(barks::Runtime::new).collect())
            .collect();
        Self {
            definition,
            handlers,
            pending: None,
            voice: None,
            retiring: false,
            completion_callbacks: 0,
        }
    }
    pub(super) fn retired(&self) -> bool {
        self.pending.is_none() && self.voice.is_none() && self.completion_callbacks == 0
    }
    pub(super) fn acknowledge_completion(&mut self) {
        if self.completion_callbacks > 0 {
            self.completion_callbacks -= 1;
        }
    }
    pub(super) fn begin_retirement(&mut self, world: &mut World, id: EntityId) -> bool {
        self.retiring = true;
        let cancelled = self.pending.take().is_some();
        if cancelled {
            self.completion_callbacks += 1;
        }
        if let Some(voice) = self.voice.as_mut() {
            if let Some(event) = voice.event.filter(|_| !voice.stopping) {
                world.write_message(super::super::audio::SoundEvent(Request::Stop {
                    event,
                    emitter: emitter(id),
                }));
                voice.stopping = true;
            } else if voice.event.is_none() {
                self.voice = None;
            }
        }
        cancelled
    }
}
fn emitter(id: EntityId) -> Emitter {
    Emitter::Equipment {
        entity: id,
        part: 8,
    }
}
fn random_range(random: &mut u32, range: [f32; 2]) -> f32 {
    *random = random.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f);
    let value = ((*random >> 10) & 0x7fff) as f32 / 32768.;
    range[0] + (range[1] - range[0]) * value
}
fn coordination(
    world: &World,
    id: EntityId,
) -> Result<
    (
        EntityId,
        crate::gameplay::pickable_component::CheckedComponent,
    ),
    String,
> {
    let worlds = world
        .get_resource::<Worlds>()
        .ok_or("Heart Barks actual AIWorld list absent")?;
    let recipient = worlds.recipient(world.resource::<LiveEntities>(), id)?;
    let checked = worlds
        .component(recipient, Slot::Coordination)?
        .ok_or("Heart Barks actual CoordinationBB absent")?;
    Ok((recipient, checked))
}
fn prune(world: &mut World, id: EntityId, now: f64) -> Result<(), String> {
    let (recipient, checked) = coordination(world, id)?;
    world.resource_scope(
        |_, mut scene: Mut<super::super::physics_scene::PhysicsScene>| {
            let owner = scene.coordination_mut(recipient, checked)?;
            owner.begin_bark_update().retain(|row| !(row.expires < now)); //5dbd90 ordered removal
            owner.finish_bark_update();
            Ok(())
        },
    )
}
struct RuleContext<'a> {
    facts: super::super::heart_context::Facts<'a>,
    history: &'a [BarkRecord],
    definition: &'a barks::Definition,
}
impl Context for RuleContext<'_> {
    fn boolean(&mut self, key: &str) -> Result<bool, String> {
        self.facts.boolean(key)
    }
    fn symbol(&mut self, key: &str) -> Result<&str, String> {
        self.facts.symbol(key)
    }
    fn area_tag(&mut self, key: &str) -> Result<u32, String> {
        self.facts.area_tag(key)
    }
    fn accepts(&mut self, outcome: &Outcome) -> Result<bool, String> {
        let Some(path) = &outcome.speech else {
            return Ok(false);
        };
        let set = self
            .definition
            .sets
            .get(path)
            .ok_or("Bark outcome Speech lease absent")?;
        Ok(!self
            .history
            .iter()
            .any(|row| row.matches(outcome.line, set)))
    }
}
fn frequency(value: &str) -> Result<usize, String> {
    Ok(match value {
        "BARK_ALWAYS" | "0" => 0,
        "BARK_OFTEN" | "1" => 1,
        "BARK_SOMETIMES" | "2" => 2,
        "BARK_RARELY" | "3" => 3,
        "BARK_ALMOST_NEVER" | "4" => 4,
        _ => return Err(format!("unknown Bark frequency {value}")),
    })
}
pub(super) fn request(
    world: &mut World,
    id: EntityId,
    target: Option<EntityId>,
    entry: &mut Entry,
) -> Result<bool, String> {
    let Some(device) = entry.device.as_mut() else {
        return Err("Heart use requires its actual AIBarks_Device component".into());
    };
    if device.retiring || entry.runtime.bark_playing {
        return Ok(false);
    }
    let target = super::super::heart_context::sanitize_target(world, target)?;
    post(
        world,
        0x50,
        id,
        Payload::BarkRequest {
            tag: Arc::from("AI_BARK_RULE_HANDLER_DEVICE"),
            trigger: 1,
            target,
        },
    )?;
    let now = world
        .resource::<ClockRuntime>()
        .clock()
        .map_err(str::to_owned)?
        .time(3)
        .ok_or("Heart BB group3 clock absent")?;
    prune(world, id, now)?;
    let old = (entry.runtime.bark_target, entry.runtime.bark_time);
    entry.runtime.bark_target = target;
    entry.runtime.bark_time = world
        .resource::<ClockRuntime>()
        .entity_time(id)
        .map_err(str::to_owned)?;
    let mut random = world
        .resource::<super::super::player::PlayerState>()
        .player
        .as_ref()
        .ok_or("Heart Barks actual Player absent")?
        .heart_random_state();
    let selection = (|| {
        let already = entry
            .runtime
            .already_asked(world.resource::<LiveEntities>());
        let facts = super::super::heart_context::Facts::read(world, id, target, already)?;
        let (recipient, checked) = coordination(world, id)?;
        let history = &world
            .resource::<super::super::physics_scene::PhysicsScene>()
            .coordination(recipient, checked)?
            .barks;
        let mut context = RuleContext {
            facts,
            history,
            definition: &device.definition,
        };
        //6123e0 explicitly requests DEVICE (143987738); only CURRENT
        //143987050 uses5dacc0's last-default-tag substitution.
        let Some(index) = device
            .definition
            .handlers
            .iter()
            .position(|(name, _)| name.as_ref() == "AI_BARK_RULE_HANDLER_DEVICE")
        else {
            return Ok(None);
        };
        for handler in &mut device.handlers[index] {
            if let Some(selected) = handler.select(&mut context, &mut random)? {
                return Ok(Some(selected));
            }
        }
        Ok(None)
    })();
    world
        .resource_mut::<super::super::player::PlayerState>()
        .player
        .as_mut()
        .ok_or("Heart Barks Player disappeared")?
        .set_heart_random_state(random);
    let selection = match selection {
        Ok(Some(value)) => value,
        other => {
            (entry.runtime.bark_target, entry.runtime.bark_time) = old;
            return other.map(|_| false);
        }
    };
    let outcome = selection.outcome();
    //5dbfb0 accepts only a strictly higher priority than the actual current
    //Speech. State66 prevents a second request while this Heart is speaking.
    let current_priority = device.voice.as_ref().map_or(0, |voice| voice.priority);
    if outcome.priority <= current_priority {
        (entry.runtime.bark_target, entry.runtime.bark_time) = old;
        return Ok(false);
    }
    if device
        .pending
        .as_ref()
        .is_some_and(|pending| pending.selection.outcome().priority >= outcome.priority)
    {
        (entry.runtime.bark_target, entry.runtime.bark_time) = old;
        return Ok(false);
    }
    if outcome.game_properties.is_some() || outcome.followup_tag.is_some() {
        (entry.runtime.bark_target, entry.runtime.bark_time) = old;
        return Err(
            "Heart selected Bark requires its actual game-properties/followup consumer".into(),
        );
    }
    let path = outcome
        .speech
        .as_ref()
        .ok_or("selected Bark has no Speech set")?;
    let set = device
        .definition
        .sets
        .get(path)
        .ok_or("selected Bark Speech set lease absent")?
        .clone();
    let corvo = super::super::player::character_name(super::super::pause_menu::debug_character())
        == "corvo";
    let recording = crate::assets::speech::selected_line_with_voice(
        set.clone(),
        outcome.line,
        device.definition.voice_resource.clone(),
        corvo,
    )?;
    let frequency = frequency(&outcome.frequency)?;
    let cooldown = random_range(&mut random, device.definition.frequency_ranges[frequency]);
    let delay = random_range(&mut random, device.definition.delay);
    if frequency != 0 {
        let (recipient, checked) = coordination(world, id)?;
        world.resource_scope(
            |_, mut scene: Mut<super::super::physics_scene::PhysicsScene>| {
                let owner = scene.coordination_mut(recipient, checked)?;
                let rows = owner.begin_bark_update();
                let row = BarkRecord {
                    line: outcome.line,
                    speech: set,
                    expires: now + cooldown as f64,
                    time_domain: 0,
                };
                if let Some(previous) = rows
                    .iter_mut()
                    .find(|previous| previous.matches(row.line, &row.speech))
                {
                    *previous = row
                } else {
                    rows.push(row)
                }
                owner.finish_bark_update();
                Ok::<_, String>(())
            },
        )?;
    }
    world
        .resource_mut::<super::super::player::PlayerState>()
        .player
        .as_mut()
        .ok_or("Heart Barks Player disappeared")?
        .set_heart_random_state(random);
    device.pending = Some(Pending {
        selection,
        recording,
        remaining: delay,
    });
    entry.runtime.bark_playing = true;
    Ok(true)
}
pub(super) fn advance(
    world: &mut World,
    id: EntityId,
    device: &mut Device,
    dt: f32,
) -> Result<(), String> {
    if let Some(pending) = device.pending.as_mut() {
        pending.remaining -= dt;
        if pending.remaining < 0. {
            let pending = device
                .pending
                .take()
                .ok_or("Heart pending Speech changed")?;
            //Retain rule generation until selected speech starts. No whole
            //handler definition is copied into this voice instance.
            let priority = pending.selection.outcome().priority;
            //A request can replace a lower-priority pending bark; an active
            //voice is retained until its real End before reuse of its emitter.
            if device.voice.is_some() {
                if let Some(voice) = device.voice.as_mut() {
                    if let Some(event) = voice.event.filter(|_| !voice.stopping) {
                        world.write_message(super::super::audio::SoundEvent(Request::Stop {
                            event,
                            emitter: emitter(id),
                        }));
                        voice.stopping = true;
                    }
                }
                device.pending = Some(pending);
            } else {
                device.voice = Some(Voice {
                    recording: pending.recording,
                    line: 0,
                    event: None,
                    stopping: false,
                    priority,
                });
            }
        }
    }
    let Some(voice) = device.voice.as_mut() else {
        return Ok(());
    };
    if let Some(event) = voice.event {
        let completion = world
            .resource_mut::<super::super::audio::SoundCompletions>()
            .take(event, emitter(id));
        let Some(success) = completion else {
            return Ok(());
        };
        voice.event = None;
        if !success {
            warn!("Heart Speech event {event:08x} did not start");
        }
        if voice.stopping || !success {
            voice.line = voice.recording.events.len()
        } else {
            voice.line += 1
        }
    }
    if let Some(&event) = voice.recording.events.get(voice.line) {
        world
            .resource_mut::<super::super::audio::SoundCompletions>()
            .watch(event, emitter(id));
        world.write_message(super::super::audio::SoundEvent(Request::Post {
            event,
            emitter: emitter(id),
        }));
        voice.event = Some(event);
    } else {
        device.voice = None;
        device.completion_callbacks += 1;
        post(world, 0xde, id, Payload::None)?; //Speech End joins the canonical FIFO.
    }
    Ok(())
}
