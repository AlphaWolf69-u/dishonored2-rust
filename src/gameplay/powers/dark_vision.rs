//! arkCpntDarkVision, the effect component the Dark Vision power starts:
//! its declaration (constructor 14049db20), the sonar and reveal pulses
//! (start 14049fe70, admission 14049f450, advance 14049f3c0) and the
//! per-frame update 14049ff50 that fills the renderer's dark-vision record
//! (renderer +0x4238..+0x426c, read by ARK_RENDERPASS_DARKVISION 14031f960,
//! ARK_RENDERPASS_DISTORTION 1403222f0 and ARK_RENDERPASS_POSTPROC 140327270).
use crate::{assets::entities::Value, gameplay::options::decl::atom};

/// One pulse block of the declaration (+0xac sonar, +0xdc reveal).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PulseSettings {
    pub initial_delay: f32,
    pub period: f32,
    pub propagation: f32,
    pub greater_propagation: f32,
    pub inner_propagation: f32,
    pub inner_greater_propagation: f32,
    pub lifetime_at_maximum: f32,
    pub fade_out: f32,
    pub maximum_radius: f32,
    pub extended_maximum_radius: f32,
    pub inner_maximum_radius: f32,
    pub inner_extended_maximum_radius: f32,
}

impl Default for PulseSettings {
    /// 14049db20's defaults, the same for both blocks.
    fn default() -> Self {
        Self {
            initial_delay: 0.1,
            period: 1.,
            propagation: 1.5,
            greater_propagation: 2.5,
            inner_propagation: 1.5,
            inner_greater_propagation: 2.5,
            lifetime_at_maximum: 1.,
            fade_out: 5.,
            maximum_radius: 15.,
            extended_maximum_radius: 25.,
            inner_maximum_radius: 15.,
            inner_extended_maximum_radius: 25.,
        }
    }
}

/// The arkDeclCpntDarkVision fields the update reads.
#[derive(Clone, Debug, PartialEq)]
pub struct DarkVisionSettings {
    /// +0x6c m_duration.
    pub duration: f32,
    /// +0x78: posted when a reveal pulse starts.
    pub pulse_sound: Option<String>,
    pub stop_sound: Option<String>,
    /// +0x88..+0x94 m_cullingDistancePerLayer, published as each layer's
    /// darkVisionDepthClip.
    pub culling_distance: [f32; 4],
    /// +0xac.
    pub sonar: PulseSettings,
    /// +0xdc.
    pub reveal: PulseSettings,
    pub vision_post_fx: Option<String>,
}

fn scope<'a>(fields: &'a [(String, Value)], name: &str) -> Option<&'a [(String, Value)]> {
    fields.iter().find_map(|(key, value)| match value {
        Value::Scope(inner) if key == name => Some(&inner[..]),
        _ => None,
    })
}

fn number(fields: &[(String, Value)], name: &str, default: f32) -> Result<f32, String> {
    match atom(fields, name) {
        None => Ok(default),
        Some(value) => value
            .trim_end_matches('f')
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("invalid Dark Vision {name}: {value}")),
    }
}

/// An idTime block (`{ m_seconds = .. }`).
fn seconds(fields: &[(String, Value)], name: &str, default: f32) -> Result<f32, String> {
    scope(fields, name).map_or(Ok(default), |inner| number(inner, "m_seconds", default))
}

fn path(fields: &[(String, Value)], name: &str) -> Option<String> {
    atom(fields, name)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
        .map(str::to_owned)
}

impl PulseSettings {
    fn read(fields: Option<&[(String, Value)]>) -> Result<Self, String> {
        let d = Self::default();
        let Some(f) = fields else { return Ok(d) };
        Ok(Self {
            initial_delay: seconds(f, "m_initialDelay", d.initial_delay)?,
            period: seconds(f, "m_period", d.period)?,
            propagation: seconds(f, "m_propagationDuration", d.propagation)?,
            greater_propagation: seconds(f, "m_greaterPropagationDuration", d.greater_propagation)?,
            inner_propagation: seconds(f, "m_innerPropagationDuration", d.inner_propagation)?,
            inner_greater_propagation: seconds(f, "m_innerGreaterPropagationDuration", d.inner_greater_propagation)?,
            lifetime_at_maximum: seconds(f, "m_lifetimeAtRadiusMax", d.lifetime_at_maximum)?,
            fade_out: seconds(f, "m_fadeOutDuration", d.fade_out)?,
            maximum_radius: number(f, "m_maximumRadius", d.maximum_radius)?,
            extended_maximum_radius: number(f, "m_extendedMaximumRadius", d.extended_maximum_radius)?,
            inner_maximum_radius: number(f, "m_innerMaximumRadius", d.inner_maximum_radius)?,
            inner_extended_maximum_radius: number(f, "m_innerExtendedMaximumRadius", d.inner_extended_maximum_radius)?,
        })
    }
}

impl DarkVisionSettings {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let mut culling_distance = [30.; 4];
        if let Some(inner) = scope(fields, "m_cullingDistancePerLayer") {
            for (layer, value) in culling_distance.iter_mut().enumerate() {
                *value = number(inner, &format!("m_cullingDistancePerLayer[{layer}]"), *value)?;
            }
        }
        Ok(Self {
            duration: seconds(fields, "m_duration", 30.)?,
            pulse_sound: path(fields, "m_sonarPulseSfx"),
            stop_sound: path(fields, "m_stopSoundEvent"),
            culling_distance,
            sonar: PulseSettings::read(scope(fields, "m_sonarPulseParameters"))?,
            reveal: PulseSettings::read(scope(fields, "m_revealPulseParameters"))?,
            vision_post_fx: path(fields, "m_visionPostFx"),
        })
    }
}

/// A running pulse (0x3c bytes at component +0x40 and +0x7c).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pulse {
    pub centre: [f32; 3],
    hold: f32,
    lifetime: f32,
    fade: f32,
    fade_out: f32,
    limit: f32,
    pub radius: f32,
    inner_limit: f32,
    pub inner_radius: f32,
    speed: f32,
    inner_speed: f32,
    pub active: bool,
}

impl Pulse {
    /// 14049fe70.
    fn start(&mut self, settings: &PulseSettings, greater: bool, centre: [f32; 3]) {
        let (limit, inner_limit, propagation, inner_propagation) = if greater {
            (settings.extended_maximum_radius, settings.inner_extended_maximum_radius,
                settings.greater_propagation, settings.inner_greater_propagation)
        } else {
            (settings.maximum_radius, settings.inner_maximum_radius,
                settings.propagation, settings.inner_propagation)
        };
        *self = Self {
            centre,
            hold: 0.,
            lifetime: settings.lifetime_at_maximum,
            fade: 0.,
            fade_out: settings.fade_out,
            limit,
            radius: 0.,
            inner_limit,
            inner_radius: 0.,
            speed: limit / propagation,
            inner_speed: inner_limit / inner_propagation,
            active: true,
        };
    }

    /// 14049f3c0.
    fn advance(&mut self, dt: f32) {
        if !self.active {
            return;
        }
        let radius = dt * self.speed + self.radius;
        let inner = dt * self.inner_speed + self.inner_radius;
        self.radius = radius;
        self.inner_radius = inner;
        if self.limit < radius {
            self.radius = self.limit;
            if self.inner_limit < inner {
                self.inner_radius = self.inner_limit;
            }
        } else if inner <= self.inner_limit {
            return;
        } else {
            self.inner_radius = self.inner_limit;
        }
        self.hold += dt;
        if self.lifetime < self.hold {
            self.fade += dt;
            if self.fade_out < self.fade {
                self.active = false;
            }
        }
    }

    /// 1 - fade / fadeOut, clamped to [0, 1] (0 fade-out reads 1).
    fn power(&self) -> f32 {
        let fraction = if self.fade_out <= 0. { 0. } else { self.fade / self.fade_out };
        (1. - fraction).clamp(0., 1.)
    }
}

/// The renderer's dark-vision record (+0x4238..+0x426c).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DarkVisionRecord {
    /// Layer bits: 1 device, 2 living, 4 pickup, 8 sonar (pulsar).
    pub mask: u32,
    /// +0x423c/+0x4240: reveal radius and inner radius.
    pub reveal_radius: [f32; 2],
    /// +0x4244/+0x4248: sonar radius and inner radius.
    pub sonar_radius: [f32; 2],
    /// +0x424c.
    pub sonar_power: f32,
    /// +0x4250.
    pub reveal_power: f32,
    /// +0x4254: the last sonar pulse centre.
    pub centre: [f32; 3],
    /// +0x4260..: darkVisionDepthClip per layer.
    pub depth_clip: [f32; 4],
}

impl DarkVisionRecord {
    /// 140310a30 inputs as the passes bind them: (inner, outer) with the
    /// inner radius at least epsilon and the outer above it.
    pub fn radii(pair: [f32; 2]) -> [f32; 2] {
        let epsilon = f32::from_bits(0x3400_0000);
        let inner = if pair[1] <= epsilon { epsilon } else { pair[1] };
        let outer = if pair[0] <= inner { inner + epsilon } else { pair[0] };
        [inner, outer]
    }
}

/// The record the render passes read this frame; None while stopped.
#[derive(bevy::prelude::Resource, Clone, Copy, Default)]
pub struct DarkVisionFrame(pub Option<DarkVisionRecord>);

/// The player attributes 14049e9d0 resolves (ARK_ATTRIBUTE_*).
#[derive(Clone, Copy, Debug, Default)]
pub struct DarkVisionAttributes {
    pub sonar_when_moving: f32,
    pub electric_field: f32,
    pub greater_distance: f32,
    pub thiefs_eyes: f32,
    pub healing: f32,
    /// MAGIC_SUPPRESSED current and base values.
    pub magic_suppressed: [f32; 2],
}

impl DarkVisionAttributes {
    pub fn read(attributes: &crate::gameplay::player::states::Attributes) -> Self {
        Self {
            sonar_when_moving: attributes.get("ARK_ATTRIBUTE_DARKVISION_SONAR_ENABLED_WHEN_MOVING"),
            electric_field: attributes.get("ARK_ATTRIBUTE_DARKVISION_ELECTRIC_FIELD"),
            greater_distance: attributes.get("ARK_ATTRIBUTE_DARKVISION_GREATER_DISTANCE"),
            thiefs_eyes: attributes.get("ARK_ATTRIBUTE_DARKVISION_THIEFS_EYES"),
            healing: attributes.get("ARK_ATTRIBUTE_DARK_VISION_HEALING"),
            magic_suppressed: [
                attributes.get("ARK_ATTRIBUTE_MAGIC_SUPPRESSED"),
                attributes.base("ARK_ATTRIBUTE_MAGIC_SUPPRESSED"),
            ],
        }
    }
}

/// What one update reports besides the record.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DarkVisionStep {
    /// The reveal pulse started: post the declaration's pulse sound.
    pub reveal_started: bool,
    /// Health to add (DARK_VISION_HEALING x dt).
    pub healing: f32,
    /// Request the component's stop (1407e7c00).
    pub stop: bool,
}

/// The running component state.
#[derive(Clone, Debug)]
pub struct DarkVisionEffect {
    pub settings: DarkVisionSettings,
    sonar: Pulse,
    reveal: Pulse,
    /// +0xe8 elapsed, +0xec reveal timer, +0xf0 sonar timer.
    elapsed: f32,
    reveal_timer: f32,
    sonar_timer: f32,
    /// The record keeps its last centre between pulses (+0x8b4 is only
    /// written when a sonar pulse starts or runs).
    centre: [f32; 3],
}

/// The player's state as the update reads it.
#[derive(Clone, Copy, Debug)]
pub struct DarkVisionPlayer {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    /// Player state +0x5d0 is 0xe (the normal playing state).
    pub playing: bool,
    /// The player's vtable+0x108 veto.
    pub veto: bool,
}

impl DarkVisionEffect {
    /// The component's start: timers and pulses from zero.
    pub fn new(settings: DarkVisionSettings) -> Self {
        Self {
            settings,
            sonar: Pulse::default(),
            reveal: Pulse::default(),
            elapsed: 0.,
            reveal_timer: 0.,
            sonar_timer: 0.,
            centre: [0.; 3],
        }
    }

    /// 14049f450: a pulse starts only while the player is still, unless the
    /// sonar works while moving.
    fn admit(pulse: &mut Pulse, settings: &PulseSettings, player: &DarkVisionPlayer, attributes: &DarkVisionAttributes) {
        let [x, y, z] = player.velocity;
        if x * x + y * y + z * z <= f32::from_bits(0x3c23_d70a) || 0. < attributes.sonar_when_moving {
            pulse.start(settings, 0. < attributes.greater_distance, player.position);
        }
    }

    /// 14049ff50 for one frame.
    pub fn update(&mut self, dt: f32, player: Option<&DarkVisionPlayer>, attributes: &DarkVisionAttributes) -> (DarkVisionRecord, DarkVisionStep) {
        let mut record = DarkVisionRecord {
            mask: 0,
            reveal_radius: [0.; 2],
            sonar_radius: [0.; 2],
            sonar_power: 1.,
            reveal_power: 1.,
            centre: self.centre,
            depth_clip: self.settings.culling_distance,
        };
        let mut step = DarkVisionStep::default();
        self.sonar_timer += dt;
        if !self.sonar.active {
            if self.settings.sonar.period < self.sonar_timer {
                if let Some(player) = player {
                    Self::admit(&mut self.sonar, &self.settings.sonar, player, attributes);
                }
                self.sonar_timer = 0.;
                self.centre = self.sonar.centre;
                record.centre = self.centre;
            }
        } else {
            self.sonar.advance(dt);
            self.centre = self.sonar.centre;
            record.centre = self.centre;
            record.sonar_radius = [self.sonar.radius, self.sonar.inner_radius];
            record.sonar_power = self.sonar.power();
        }
        self.reveal_timer += dt;
        if !self.reveal.active {
            if self.settings.reveal.period < self.reveal_timer {
                if let Some(player) = player {
                    Self::admit(&mut self.reveal, &self.settings.reveal, player, attributes);
                }
                self.reveal_timer = 0.;
                step.reveal_started = self.settings.pulse_sound.is_some();
            }
        } else {
            self.reveal.advance(dt);
            record.reveal_power = self.reveal.power();
            record.reveal_radius = [self.reveal.radius, self.reveal.inner_radius];
        }
        if attributes.healing != 0. {
            step.healing = dt * attributes.healing;
        }
        self.elapsed += dt;
        let mut mask = 10;
        if 0. < attributes.electric_field {
            mask = 11;
        }
        if 0. < attributes.thiefs_eyes {
            mask |= 4;
        }
        record.mask = mask;
        step.stop = match player {
            None => true,
            Some(player) => !player.playing
                || attributes.magic_suppressed[0] != attributes.magic_suppressed[1]
                || self.settings.duration <= self.elapsed
                || player.veto,
        };
        (record, step)
    }
}
