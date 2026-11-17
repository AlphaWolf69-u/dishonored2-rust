//! Heart declaration611800 and retained target/pulse state613170/612cd0.
//! Sensor ownership, audio and Midnight playback stay with their real owners.
use crate::{
    assets::entities::{
        Value,
        scene_motion::{atom, child},
    },
    gameplay::{damage::number, entity_lifetime::EntityId},
};
use std::{collections::VecDeque, sync::Arc};
pub mod barks;
pub mod sensor;

type Fields = [(String, Value)];
#[derive(Clone, Copy, Debug)]
pub struct Rumble {
    pub low: f32,
    pub high: f32,
    pub duration: f32,
}
impl Rumble {
    fn read(fields: &Fields) -> Result<Self, String> {
        Ok(Self {
            low: number(fields, "m_lowIntensity", 0.1)?,
            high: number(fields, "m_highIntensity", 0.2)?,
            duration: number(fields, "m_rumbleDuration", 0.5)?,
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Heartbeat {
    pub lub_dub_ms: f32,
    pub period_ms: f32,
    pub rumble: Rumble,
    pub distance: f32,
}
impl Heartbeat {
    fn read(fields: &Fields) -> Result<Self, String> {
        let pulse = child(fields, "m_pulse");
        Ok(Self {
            lub_dub_ms: number(pulse, "m_lubDubperiod", 200.)?,
            period_ms: number(pulse, "m_pulsePeriod", 600.)?,
            rumble: Rumble::read(child(fields, "m_rumble"))?,
            distance: number(fields, "m_distance", 50.)?,
        })
    }
}
#[derive(Clone, Debug)]
pub struct Definition {
    pub lumb_rumble: Rumble,
    pub heartbeats: Vec<Heartbeat>,
    pub slow_playrate: f32,
    pub fast_playrate: f32,
    pub slow_period: f32,
    pub fast_period: f32,
    pub equip_sound: Option<String>,
    pub unequip_sound: Option<String>,
    pub angle_for_target: f32,
    pub bark_interval: f32,
}
impl Definition {
    pub fn read(fields: &Fields) -> Result<Self, String> {
        let optional = |key| {
            atom(fields, key)
                .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
                .map(str::to_owned)
        };
        let list = child(fields, "m_heartBeatKeyPoint");
        //611660 constructs an empty fixed-capacity4 list, with initialized
        //backing values. An omitted list never invents active keypoints.
        let count = atom(list, "num")
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| "invalid Heart keypoint count")?;
        if count > 4 {
            return Err("Heart keypoint count exceeds native fixed capacity4".into());
        }
        let heartbeats = (0..count)
            .map(|index| Heartbeat::read(child(list, &format!("item[{index}]"))))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            lumb_rumble: Rumble::read(child(fields, "m_lumbRumble"))?,
            heartbeats,
            slow_playrate: number(fields, "m_slowplayrate", 0.1)?,
            fast_playrate: number(fields, "m_fastplayrate", 2.)?,
            slow_period: number(fields, "m_heartPeriodForSlowPlayrate", 1.)?,
            fast_period: number(fields, "m_heartPeriodForFastPlayrate", 0.05)?,
            equip_sound: optional("m_equipSound"),
            unequip_sound: optional("m_unequipSound"),
            angle_for_target: number(fields, "m_angleForTarget", 20.)?,
            bark_interval: number(fields, "m_minimalIntervalForBarks", 5.)?,
        })
    }
    pub fn sound_paths(&self) -> impl Iterator<Item = &str> {
        self.equip_sound
            .as_deref()
            .into_iter()
            .chain(self.unequip_sound.as_deref())
    }
    ///611da0's comparisons deliberately map a NaN quotient to0.
    pub fn playrate(&self, period: f32) -> f32 {
        let ratio = (self.slow_period - period) / (self.slow_period - self.fast_period);
        let ratio = if ratio >= 0. { ratio.min(1.) } else { 0. };
        (1. - ratio) * self.slow_playrate + ratio * self.fast_playrate
    }
}
/// One borrowed RuneCharms sensor20 result, preserving its managed generation.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub entity: EntityId,
    pub angle: f32,
    pub distance: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct UseRequest {
    pub equipment: EntityId,
    pub target: Option<EntityId>,
}
impl crate::gameplay::player::Player {
    pub fn heart_bark_status(&mut self, playing: bool) {
        self.heart_bark_playing = playing;
    }
    pub fn heart_use_request(&self) -> Option<UseRequest> {
        self.heart_requests.front().copied()
    }
    pub fn acknowledge_heart_use(&mut self) {
        self.heart_requests.pop_front();
    }
    pub fn heart_random_state(&self) -> u32 {
        self.random
    }
    pub fn set_heart_random_state(&mut self, state: u32) {
        self.random = state;
    }
    pub fn bind_heart_entity(&mut self, entity: Option<EntityId>) {
        self.heart_entity = entity;
    }
    pub fn heart_entity(&self) -> Option<EntityId> {
        self.heart_entity
    }
    pub fn equip_heart_state(&mut self, equipped: bool) {
        if equipped {
            if !self
                .machine
                .is_active(crate::gameplay::player::states::StateId::USE_HEART)
            {
                self.machine
                    .request(crate::gameplay::player::states::StateId::USE_HEART)
            }
        } else {
            self.machine
                .quit(crate::gameplay::player::states::StateId::USE_HEART)
        }
    }
    ///The currently hosted modes are default and swimming;802de0's default
    ///mode remains distinct from the possession-controller enum.
    pub fn heart_default_mode(&self) -> bool {
        !self.swimming_active
    }
    pub fn set_animation_scene_rate(&mut self, handle: u32, rate: f32) -> Result<bool, String> {
        self.body.midnight.set_rate(handle, rate)
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Effect {
    Rumble(Rumble), //normal numeric11a, three floats
    Playrate(f32),  //normal numeric11d, one float
    TargetLost,     //normal numeric11b(false)
}
pub struct Runtime {
    pub definition: Arc<Definition>,
    pub equipped: bool,
    pub target: Option<Target>,
    pub bark_target: Option<EntityId>,
    pub bark_time: f64,
    pub bark_playing: bool,
    seen_targets: Vec<EntityId>,
    last_lub: f64,
    pub last_tick: f64,
    dub_pending: bool,
    lub_period: f32,
    effects: VecDeque<Effect>,
}
impl Runtime {
    pub fn new(definition: Arc<Definition>) -> Self {
        Self {
            definition,
            equipped: false,
            target: None,
            bark_target: None,
            bark_time: 0.,
            bark_playing: false,
            seen_targets: Vec::new(),
            last_lub: 0.,
            last_tick: 0.,
            dub_pending: false,
            lub_period: 0.,
            effects: VecDeque::new(),
        }
    }
    ///6121e0 resets pulse/target state on actual Equip, not on admission.
    pub fn equip(&mut self) {
        self.equipped = true;
        self.bark_playing = false; //6121e0 clears Heart+104 bit0.
        self.target = None;
        self.dub_pending = false;
    }
    ///6122d0 withdraws the target and persistent UseHeart state on Unequip.
    pub fn unequip(&mut self) {
        self.equipped = false;
        self.target = None;
    }
    ///613170 chooses the smallest angle first, then applies the angle gate.
    pub fn select_target(&mut self, rows: impl IntoIterator<Item = Target>, now: f64) {
        if !self.equipped {
            return;
        }
        let mut selected: Option<Target> = None;
        for row in rows {
            if row.angle < selected.map_or(f32::from_bits(0x7149f2ca), |old| old.angle) {
                selected = Some(row);
            }
        }
        selected = selected.filter(|row| {
            row.angle <= self.definition.angle_for_target * (std::f32::consts::PI / 180.)
        });
        if selected.is_some() && self.target.is_none() {
            //612ae0 starts the pulse clock and publishes the current rate.
            self.effects
                .push_back(Effect::Playrate(self.definition.playrate(self.lub_period)));
            self.last_lub = now;
            self.last_tick = now;
            self.dub_pending = false;
        } else if selected.is_none() && self.target.is_some() {
            self.effects.push_back(Effect::TargetLost);
        }
        self.target = selected;
    }
    ///612cd0 interpolates the two authored bounding distance records. Tick
    ///is gated by the actual game/default-controller state at its caller.
    pub fn update(&mut self, now: f64, enabled: bool) {
        if !self.equipped || !enabled {
            return;
        }
        let Some(target) = self.target else {
            return;
        };
        let mut lower: Option<&Heartbeat> = None;
        let mut upper: Option<&Heartbeat> = None;
        for point in &self.definition.heartbeats {
            if target.distance <= point.distance
                && upper.is_none_or(|old| point.distance < old.distance)
            {
                upper = Some(point);
            }
            if point.distance <= target.distance
                && lower.is_none_or(|old| old.distance < point.distance)
            {
                lower = Some(point);
            }
        }
        let (Some(lower), Some(upper)) = (lower, upper) else {
            self.dub_pending = false;
            return;
        };
        let ratio = (target.distance - lower.distance) / (upper.distance - lower.distance);
        let ratio = if ratio >= 0. { ratio.min(1.) } else { 0. };
        let interpolate = |left: f32, right: f32| (1. - ratio) * left + ratio * right;
        self.lub_period = interpolate(lower.lub_dub_ms, upper.lub_dub_ms) / 1000.;
        let period = interpolate(lower.period_ms, upper.period_ms) / 1000.;
        let elapsed = (now - self.last_lub) as f32;
        if !self.dub_pending && self.lub_period <= elapsed {
            self.effects.push_back(Effect::Rumble(Rumble {
                low: interpolate(lower.rumble.low, upper.rumble.low),
                high: interpolate(lower.rumble.high, upper.rumble.high),
                duration: interpolate(lower.rumble.duration, upper.rumble.duration) / 1000.,
            }));
            self.dub_pending = true;
        }
        if period <= elapsed {
            self.dub_pending = false;
            self.last_lub = now;
            self.effects
                .push_back(Effect::Rumble(self.definition.lumb_rumble));
            self.effects
                .push_back(Effect::Playrate(self.definition.playrate(self.lub_period)));
        }
        self.last_tick = now;
    }
    pub fn pending(&self) -> Option<Effect> {
        self.effects.front().copied()
    }
    pub fn acknowledge(&mut self) {
        self.effects.pop_front();
    }
    pub fn already_asked(&self, live: &crate::gameplay::entity_lifetime::LiveEntities) -> bool {
        self.bark_target
            .is_some_and(|target| live.get(target).is_some() && self.seen_targets.contains(&target))
    }
    ///611e10 clears the playing bit and remembers only the still-live target.
    pub fn bark_finished(&mut self, live: &crate::gameplay::entity_lifetime::LiveEntities) {
        self.bark_playing = false;
        if let Some(target) = self
            .bark_target
            .filter(|target| live.get(*target).is_some())
        {
            if !self.seen_targets.contains(&target) {
                self.seen_targets.push(target);
            }
        }
    }
}
