//! Authored FX groups, environment blends and sound descriptors. Serialized
//! fields are partial edits of their class constructor defaults.
use super::entities::Value;
use crate::gameplay::effects::{curve::*, environment::EnvironmentEffectDefinition};
pub(crate) mod declaration;
pub(crate) mod definition;
pub mod particle_system;
pub mod particle_declaration;
pub(crate) mod particle_resources;
mod particle_emission_mesh;
pub mod particle_mesh;
pub mod particle_sprite;
pub mod particle_stage;
pub mod particle_attractor;
pub mod particle_trail;
pub mod particle_beam;
pub mod particle_ribbon;
pub mod particle_havok_stage;
pub mod particle_field;
pub mod particles;
pub mod screen_quad;
pub mod decal;
pub mod fade;
pub mod entity_def;
pub mod explosion;
pub mod light;
pub mod screen_distortion;
pub mod fov;
pub mod rumble;
pub mod screen_fade;

/// arkFXDescStructures::arkStandardParameter<float>: a sampled range
/// (max, min) and the duration, fade-in and fade-out curve pairs.
#[derive(Clone)]
pub struct StandardParameter {
    pub scale: [f32; 2],
    pub duration: crate::gameplay::effects::curve::EffectFloatCurvePair,
    pub fade_in: crate::gameplay::effects::curve::EffectFloatCurvePair,
    pub fade_out: crate::gameplay::effects::curve::EffectFloatCurvePair,
}
impl StandardParameter {
    /// 1402262d0(value, curve): the range (value, value) and each curve's
    /// maximum a single key of `curve`.
    fn constant(value: f32, curve: f32) -> Self {
        let flat = || crate::gameplay::effects::curve::EffectFloatCurvePair {
            maximum: crate::gameplay::effects::curve::EffectFloatCurve { times: vec![0.], values: vec![0., curve, 0.] },
            minimum: Default::default(),
        };
        Self { scale: [value; 2], duration: flat(), fade_in: flat(), fade_out: flat() }
    }
    fn read(&mut self, fields: &Fields) -> Result<(), String> {
        for (name, value) in fields {
            match name.as_str() {
                "m_max" => self.scale[0] = number(value)?,
                "m_min" => self.scale[1] = number(value)?,
                "m_durationCurve" => curves(&mut self.duration, scope(value)?)?,
                "m_fadeinCurve" => curves(&mut self.fade_in, scope(value)?)?,
                "m_fadeoutCurve" => curves(&mut self.fade_out, scope(value)?)?,
                _ => return Err(format!("unknown FX parameter field {name}")),
            }
        }
        Ok(())
    }
}

/// arkFXCameraShakeDesc::shakeDesc_t (constructor 14022ab20): amplitude
/// (default 0.05) and frequency (default 10 Hz), flat curves.
#[derive(Clone)]
pub struct ShakeOscillator {
    pub amplitude: StandardParameter,
    pub frequency: StandardParameter,
}
fn shake_oscillator(fields: &Fields) -> Result<ShakeOscillator, String> {
    let mut oscillator = ShakeOscillator {
        amplitude: StandardParameter::constant(f32::from_bits(0x3d4c_cccd), 1.),
        frequency: StandardParameter::constant(10., 1.),
    };
    for (name, value) in fields {
        match name.as_str() {
            "m_amplitude" => oscillator.amplitude.read(scope(value)?)?,
            "m_frequency" => oscillator.frequency.read(scope(value)?)?,
            _ => return Err(format!("unknown FX shake oscillator field {name}")),
        }
    }
    Ok(oscillator)
}

/// arkFXCameraShakeDesc (constructor 14022aba0): standard timings, the
/// global magnitude (1, flat), positional and angular channels (X, Y, Z,
/// toward the origin), the rotation order (default 5), the distance
/// radius (10) and fade, and the impulse range (25) and fade.
pub struct CameraShakeAsset {
    pub name: String,
    pub enabled: bool,
    pub looping: bool,
    pub keep_duration_on_stop: bool,
    pub timing: crate::gameplay::effects::timing::EffectTimingRanges,
    pub magnitude: StandardParameter,
    /// Positional X, Y, Z, origin; angular X, Y, Z, origin.
    pub channels: [Vec<ShakeOscillator>; 8],
    pub rotation_order: u32,
    pub distance_radius: [f32; 2],
    pub distance_fade: crate::gameplay::effects::curve::EffectFloatCurvePair,
    pub impulse: [f32; 2],
    pub impulse_fade: crate::gameplay::effects::curve::EffectFloatCurvePair,
}
pub fn camera_shake(fields: &Fields) -> Result<CameraShakeAsset, String> {
    let fade = || crate::gameplay::effects::curve::EffectFloatCurvePair {
        maximum: crate::gameplay::effects::curve::EffectFloatCurve { times: vec![0., 1.], values: vec![0., 1., 0., 0., 0., 0.] },
        minimum: Default::default(),
    };
    let mut shake = CameraShakeAsset {
        name: String::new(), enabled: true, looping: false, keep_duration_on_stop: false,
        timing: Default::default(),
        magnitude: StandardParameter::constant(1., 1.),
        channels: Default::default(),
        rotation_order: 5,
        distance_radius: [10.; 2],
        distance_fade: fade(),
        impulse: [25.; 2],
        impulse_fade: fade(),
    };
    let channel_names = ["m_X", "m_Y", "m_Z", "m_origin"];
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => shake.name = atom(value)?.to_owned(),
            "m_enabled" => shake.enabled = boolean(value)?,
            "m_timings" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_looping" => shake.looping = boolean(value)?,
                        "m_finishLoopOnStop" => shake.keep_duration_on_stop = boolean(value)?,
                        "m_delay" => bounds(&mut shake.timing.delay, scope(value)?)?,
                        "m_fadein" => bounds(&mut shake.timing.fade_in, scope(value)?)?,
                        "m_duration" => bounds(&mut shake.timing.duration, scope(value)?)?,
                        "m_fadeout" => bounds(&mut shake.timing.fade_out, scope(value)?)?,
                        _ => return Err(format!("unknown FX timing field {name}")),
                    }
                }
            }
            "m_globalMagnitude" => shake.magnitude.read(scope(value)?)?,
            "m_positional" | "m_angular" => {
                let base = if name == "m_positional" { 0 } else { 4 };
                for (field, value) in scope(value)? {
                    if field == "m_rotationOrder" && base == 4 {
                        shake.rotation_order = rotation_order(atom(value)?)?;
                        continue;
                    }
                    let index = channel_names.iter().position(|channel| channel == field)
                        .ok_or_else(|| format!("unknown FX shake channel {field}"))?;
                    descriptor_list(&mut shake.channels[base + index], scope(value)?, shake_oscillator)?;
                }
            }
            "m_distanceRadius" => bounds(&mut shake.distance_radius, scope(value)?)?,
            "m_impulse" => bounds(&mut shake.impulse, scope(value)?)?,
            "m_distanceFade" | "m_impulseFade" => {
                let target = if name == "m_distanceFade" { &mut shake.distance_fade } else { &mut shake.impulse_fade };
                for (field, value) in scope(value)? {
                    if field != "m_curve" {
                        return Err(format!("unknown FX curve field {field}"));
                    }
                    curves(target, scope(value)?)?;
                }
            }
            _ => return Err(format!("unknown FX camera shake field {name}")),
        }
    }
    Ok(shake)
}
/// idAngles::RotateOrder by name or number.
fn rotation_order(value: &str) -> Result<u32, String> {
    Ok(match value {
        "XYZ" | "ROTATE_XYZ" => 0, "YZX" | "ROTATE_YZX" => 1, "ZXY" | "ROTATE_ZXY" => 2,
        "XZY" | "ROTATE_XZY" => 3, "YXZ" | "ROTATE_YXZ" => 4, "ZYX" | "ROTATE_ZYX" => 5,
        other => other.parse().map_err(|_| format!("unknown rotation order {other}"))?,
    })
}

pub struct EnvironmentEffectAsset {
    pub name: String,
    pub enabled: bool,
    pub environment: Option<String>,
    pub blend: std::sync::Arc<EnvironmentEffectDefinition>,
}

/// The location enum's literal table is initialized by 1419fbd70. Eligibility
/// for the current view is decided by the parent FX owner, not by this loader.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EffectGroupLocation {
    #[default]
    Everywhere,
    OutOfWater,
    Underwater,
}

pub struct EffectGroupAsset {
    pub name: String,
    pub enabled: bool,
    pub location: EffectGroupLocation,
    pub timings: crate::gameplay::effects::timing::EffectTimingRanges,
    pub environments: Vec<EnvironmentEffectAsset>,
    pub sounds: Vec<SoundEffectAsset>,
    pub sub_effects: Vec<SubEffectAsset>,
    pub entity_defs: Vec<entity_def::EntityDefAsset>,
    pub explosions: Vec<explosion::ExplosionAsset>,
    pub camera_shakes: Vec<std::sync::Arc<CameraShakeAsset>>,
    pub screen_fades: Vec<std::sync::Arc<screen_fade::ScreenFadeAsset>>,
    /// m_skinDecals (group +0x70, arkFXSkinDecalDesc 0x70 bytes).
    pub skin_decals: Vec<std::sync::Arc<SkinDecalAsset>>,
    /// m_frameCaptures (arkFXFrameCaptureDesc, timed by 21a870/21a730).
    /// With g_stopTime clear, 140323e70 copies the tone-mapped scene into
    /// `_frameCapture` every frame, so these events change nothing that
    /// normal play renders; they are kept for their schedule only.
    pub frame_captures: Vec<crate::gameplay::effects::standard_events::StandardEventTimings>,
}
pub type EnvironmentGroupAsset = EffectGroupAsset;

/// arkFXSubFXDesc: nested FX played by a group (group+0x120, 0x38 bytes).
/// Ranges keep the serialized maximum-then-minimum order of arkFXMinMax.
pub struct SubEffectAsset {
    /// arkSubFXDesc +0: m_name (idAtomicString).
    pub name: String,
    pub enabled: bool,
    /// arkStandardEventTimings: m_delay, m_cycles and m_period.
    pub delay: [f32; 2],
    pub cycles: [i32; 2],
    pub period: [f32; 2],
    pub wait_end: bool,
    pub sequential: bool,
    pub random_start: bool,
    /// m_decls: FX resources; None for a NULL entry.
    pub effects: Vec<Option<String>>,
}

impl Default for SubEffectAsset {
    /// 230500/2242d0 constructor defaults: enabled, m_waitFXEnd set,
    /// m_sequentialMode and m_randomStart clear; delay 0, one cycle, period 1.
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            delay: [0.; 2],
            cycles: [1; 2],
            period: [1.; 2],
            wait_end: true,
            sequential: false,
            random_start: false,
            effects: Vec::new(),
        }
    }
}

fn sub_effect(fields: &Fields) -> Result<SubEffectAsset, String> {
    let mut effect = SubEffectAsset::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => effect.name = atom(value)?.to_owned(),
            "m_enabled" => effect.enabled = boolean(value)?,
            "m_waitFXEnd" => effect.wait_end = boolean(value)?,
            "m_sequentialMode" => effect.sequential = boolean(value)?,
            "m_randomStart" => effect.random_start = boolean(value)?,
            "m_timings" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_delay" => bounds(&mut effect.delay, scope(value)?)?,
                        "m_period" => bounds(&mut effect.period, scope(value)?)?,
                        "m_cycles" => {
                            let mut pair = effect.cycles.map(|v| v as f32);
                            bounds(&mut pair, scope(value)?)?;
                            effect.cycles = pair.map(|v| v as i32);
                        }
                        _ => return Err(format!("unknown sub-FX timing field {name}")),
                    }
                }
            }
            "m_decls" => {
                for (position, (name, value)) in scope(value)?.iter().enumerate() {
                    if name == "num" {
                        if position != 0 {
                            return Err("sub-FX declaration count must precede its elements".into());
                        }
                        let count = atom(value)?.parse::<usize>().map_err(|e| format!("sub-FX count: {e}"))?;
                        effect.effects.resize(count, None);
                    } else {
                        let index = name.strip_prefix("item[").and_then(|n| n.strip_suffix(']'))
                            .ok_or_else(|| format!("unknown sub-FX declaration field {name}"))?
                            .parse::<usize>().map_err(|e| format!("sub-FX index: {e}"))?;
                        let entry = effect.effects.get_mut(index)
                            .ok_or_else(|| format!("sub-FX declaration {index} exceeds declared count"))?;
                        let path = atom(value)?;
                        *entry = (!path.is_empty() && !path.eq_ignore_ascii_case("NULL")).then(|| path.to_owned());
                    }
                }
            }
            _ => return Err(format!("unknown sub-FX field {name}")),
        }
    }
    Ok(effect)
}

pub struct SoundEventAsset {
    /// Serialized order: maximum followed by minimum.
    pub delay: [f32; 2],
    pub event: Option<String>,
}
impl SoundEventAsset {
    /// Resolve the installed declaration, including inherited edits, once for
    /// both contact FX and ordinary world FX sound-object preparation.
    pub fn resolve(&self, catalog: &crate::archive::Catalog) -> Result<Option<std::sync::Arc<crate::assets::wwise::SoundEventDeclaration>>, String> {
        self.resolve_with(catalog, |path| {
            catalog.read(path)
                .map(|bytes| bytes.map(crate::entities::owned_declaration_text))
                .map_err(|error| format!("{path}: {error}"))
        })
    }

    /// [`Self::resolve`] with declaration text from `read`.
    pub fn resolve_with(&self, catalog: &crate::archive::Catalog, read: impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Option<std::sync::Arc<crate::assets::wwise::SoundEventDeclaration>>, String> {
        let Some(name) = self.event.as_deref() else { return Ok(None) };
        crate::assets::wwise::event_resources::acquire_with(catalog, name, read).map(Some)
    }
}

pub struct SoundEffectAsset {
    pub name: String,
    pub enabled: bool,
    pub position: [bevy_math::Vec3; 2],
    pub start: SoundEventAsset,
    pub stop: SoundEventAsset,
}

impl Default for EffectGroupAsset {
    fn default() -> Self {
        // 2174a0 resets group ranges independently of StandardTimings: group
        // duration is one, while delay, fade-in and fade-out are zero.
        Self {
            name: String::new(),
            enabled: true,
            location: EffectGroupLocation::Everywhere,
            timings: crate::gameplay::effects::timing::EffectTimingRanges {
                duration: [1.; 2],
                ..Default::default()
            },
            environments: Vec::new(),
            sounds: Vec::new(),
            sub_effects: Vec::new(),
            entity_defs: Vec::new(),
            explosions: Vec::new(),
            camera_shakes: Vec::new(),
            screen_fades: Vec::new(),
            skin_decals: Vec::new(),
            frame_captures: Vec::new(),
        }
    }
}

type Fields = [(String, Value)];
/// arkFXSkinDecalDesc (reflection 1415067a0, constructor 22dba0). Only the
/// fields the family runtime reads are kept: the start (22e8e0) samples
/// m_delay and its gate (22dcb0) requires a texture. The projection fields
/// feed 22e180, which runs only for a skinned target.
pub struct SkinDecalAsset {
    pub name: String,
    pub enabled: bool,
    pub textures: Vec<String>,
    /// [m_max, m_min]; 22dba0 clears it.
    pub delay: [f32; 2],
}

pub fn skin_decal(fields: &Fields) -> Result<SkinDecalAsset, String> {
    let mut decal = SkinDecalAsset { name: String::new(), enabled: true, textures: Vec::new(), delay: [0.; 2] };
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => decal.name = atom(value)?.to_owned(),
            "m_enabled" => decal.enabled = boolean(value)?,
            "m_textures" => {
                for (field, value) in scope(value)? {
                    if field == "num" { continue; }
                    if !field.starts_with("item[") { return Err(format!("unknown FX skin decal texture field {field}")); }
                    decal.textures.push(atom(value)?.trim_matches('"').to_owned());
                }
            }
            "m_delay" => bounds(&mut decal.delay, scope(value)?)?,
            "m_projectionAxis" => match atom(value)?.trim_matches('"') {
                "X" | "Y" | "Z" => {}
                other => return Err(format!("unknown FX skin decal axis {other}")),
            },
            "m_projectionDepth" | "m_projectionDepthOffset" | "m_initialRotation" => bounds(&mut [0.; 2], scope(value)?)?,
            "m_initialPosition" | "m_size" => {}
            _ => return Err(format!("unknown FX skin decal field {name}")),
        }
    }
    Ok(decal)
}

fn scope(value: &Value) -> Result<&Fields, String> {
    match value {
        Value::Scope(fields) => Ok(fields),
        _ => Err("FX field requires a scope".into()),
    }
}
fn atom(value: &Value) -> Result<&str, String> {
    match value {
        Value::Atom(value) => Ok(value.trim_matches('"')),
        _ => Err("FX field requires an atom".into()),
    }
}
fn number(value: &Value) -> Result<f32, String> {
    atom(value)?
        .parse()
        .map_err(|e| format!("invalid FX float: {e}"))
}
fn boolean(value: &Value) -> Result<bool, String> {
    atom(value)?
        .parse()
        .map_err(|e| format!("invalid FX boolean: {e}"))
}
fn bounds(pair: &mut [f32; 2], fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_max" => pair[0] = number(value)?,
            "m_min" => pair[1] = number(value)?,
            _ => return Err(format!("unknown FX range field {name}")),
        }
    }
    Ok(())
}

pub(crate) fn descriptor_list<'a, T>(list: &mut Vec<T>, fields: &'a Fields, read: fn(&'a Fields) -> Result<T, String>) -> Result<(), String> {
    for (position, (name, value)) in fields.iter().enumerate() {
        if name == "num" {
            if position != 0 { return Err("FX descriptor count must precede its elements".into()); }
            let count = atom(value)?.parse::<usize>().map_err(|e| format!("FX descriptor count: {e}"))?;
            list.truncate(count);
            while list.len() < count { list.push(read(&[])?); }
        } else {
            let index = name.strip_prefix("item[").and_then(|n| n.strip_suffix(']'))
                .ok_or_else(|| format!("unknown FX descriptor list field {name}"))?
                .parse::<usize>().map_err(|e| format!("FX descriptor index: {e}"))?;
            let entry = list.get_mut(index).ok_or_else(|| format!("FX descriptor {index} exceeds declared count"))?;
            *entry = read(scope(value)?)?;
        }
    }
    Ok(())
}

fn sound_event(fields: &Fields) -> Result<SoundEventAsset, String> {
    let mut event = SoundEventAsset { delay: [0.0; 2], event: None };
    // soundEvent_t reflection 141506c90; 22f560 clears both event records.
    for (name, value) in fields {
        match name.as_str() {
            "m_delay" => bounds(&mut event.delay, scope(value)?)?,
            "m_soundEvent" => {
                let name = atom(value)?;
                event.event = (!name.is_empty() && !name.eq_ignore_ascii_case("NULL")).then(|| name.to_owned());
            }
            _ => return Err(format!("unknown FX sound event field {name}")),
        }
    }
    Ok(event)
}

/// arkFXSoundDesc defaults (22f540 / 22f560) and reflected fields (1506de0).
pub fn sound_effect(fields: &Fields) -> Result<SoundEffectAsset, String> {
    let mut sound = SoundEffectAsset {
        name: String::new(), enabled: true, position: [bevy_math::Vec3::ZERO; 2],
        start: sound_event(&[])?, stop: sound_event(&[])?,
    };
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => sound.name = atom(value)?.to_owned(),
            "m_enabled" => sound.enabled = boolean(value)?,
            "m_startSoundEvent" => sound.start = sound_event(scope(value)?)?,
            "m_stopSoundEvent" => sound.stop = sound_event(scope(value)?)?,
            "m_position" => {
                for (name, value) in scope(value)? {
                    let slot = match name.as_str() {
                        "m_max" => &mut sound.position[0],
                        "m_min" => &mut sound.position[1],
                        _ => return Err(format!("unknown FX position range field {name}")),
                    };
                    let position = scope(value)?;
                    for (name, value) in position {
                        let axis = match name.as_str() { "x" => 0, "y" => 1, "z" => 2, _ => return Err(format!("unknown FX position component {name}")) };
                        slot[axis] = number(value)?;
                    }
                }
            }
            _ => return Err(format!("unknown sound FX field {name}")),
        }
    }
    Ok(sound)
}
fn float_list(values: &mut Vec<f32>, fields: &Fields) -> Result<(), String> {
    edit_list(values, fields, 0., |slot, value| {
        *slot = number(value)?;
        Ok(())
    })
}
fn edit_list<T: Clone>(
    values: &mut Vec<T>,
    fields: &Fields,
    initial: T,
    mut edit: impl FnMut(&mut T, &Value) -> Result<(), String>,
) -> Result<(), String> {
    let mut edited = values.clone();
    if let Some((_, count)) = fields.first().filter(|(name, _)| name == "num") {
        let count = atom(count)?
            .parse::<usize>()
            .map_err(|e| format!("FX array count: {e}"))?;
        // 98c270 -> 14d230 classifies float as a zero-initialized element;
        // 9979c0 preserves old entries and clears newly exposed storage.
        edited.resize(count, initial);
    }
    for (position, (name, value)) in fields.iter().enumerate() {
        if name == "num" {
            if position == 0 {
                continue;
            }
            return Err("FX array count must precede its elements".into());
        }
        let index = name
            .strip_prefix("item[")
            .and_then(|n| n.strip_suffix(']'))
            .ok_or_else(|| format!("unknown FX array field {name}"))?
            .parse::<usize>()
            .map_err(|e| format!("FX array index: {e}"))?;
        // 14014d300 warns "array element %d out of range" and skips an
        // element past the current count; it does not grow the list.
        let Some(slot) = edited.get_mut(index) else { continue };
        edit(slot, value)?;
    }
    *values = edited;
    Ok(())
}
fn curve(curve: &mut EffectFloatCurve, fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_times" => float_list(&mut curve.times, scope(value)?)?,
            "m_values" => float_list(&mut curve.values, scope(value)?)?,
            // 1cbd50 samples the packed arrays directly; these generic curve
            // settings/tangent caches are not read by the scalar FX sampler.
            "m_loopMode" | "m_interpolationMode" | "m_tangentsIn" | "m_tangentsOut" => {}
            _ => return Err(format!("unknown FX curve field {name}")),
        }
    }
    Ok(())
}
fn curves(pair: &mut EffectFloatCurvePair, fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_maxCurve" => curve(&mut pair.maximum, scope(value)?)?,
            "m_minCurve" => curve(&mut pair.minimum, scope(value)?)?,
            _ => return Err(format!("unknown FX curve-pair field {name}")),
        }
    }
    Ok(())
}

/// Load one `m_envOverrides/item[N]`. The parent group retains ownership of its
/// own timing, activation and visibility mask; this loader does not start it.
pub fn environment_effect(fields: &Fields) -> Result<EnvironmentEffectAsset, String> {
    environment_effect_fields(fields)
}
fn environment_effect_fields<'a>(fields: impl IntoIterator<Item = &'a (String, Value)>) -> Result<EnvironmentEffectAsset, String> {
    let mut effect = EnvironmentEffectAsset {
        name: String::new(),
        enabled: true,
        environment: None,
        blend: std::sync::Arc::new(EnvironmentEffectDefinition::default()),
    };
    let blend = std::sync::Arc::get_mut(&mut effect.blend).expect("unpublished FX blend");
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => effect.name = atom(value)?.to_owned(),
            "m_enabled" => effect.enabled = boolean(value)?,
            "m_decl" => {
                let name = atom(value)?;
                effect.environment = (!name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
                    .then(|| name.to_owned());
            }
            "m_timings" => {
                for (name, value) in scope(value)? {
                    let timing = &mut blend.timing;
                    match name.as_str() {
                        "m_looping" => blend.looping = boolean(value)?,
                        "m_finishLoopOnStop" => {
                            blend.keep_duration_on_stop = boolean(value)?
                        }
                        "m_delay" => bounds(&mut timing.delay, scope(value)?)?,
                        "m_fadein" => bounds(&mut timing.fade_in, scope(value)?)?,
                        "m_duration" => bounds(&mut timing.duration, scope(value)?)?,
                        "m_fadeout" => bounds(&mut timing.fade_out, scope(value)?)?,
                        _ => return Err(format!("unknown FX timing field {name}")),
                    }
                }
            }
            "m_blend" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_max" => blend.scale[0] = number(value)?,
                        "m_min" => blend.scale[1] = number(value)?,
                        "m_durationCurve" => curves(&mut blend.duration, scope(value)?)?,
                        "m_fadeinCurve" => curves(&mut blend.fade_in, scope(value)?)?,
                        "m_fadeoutCurve" => curves(&mut blend.fade_out, scope(value)?)?,
                        _ => return Err(format!("unknown FX blend field {name}")),
                    }
                }
            }
            _ => return Err(format!("unknown environment FX field {name}")),
        }
    }
    Ok(effect)
}

/// Load the environment portion of one `m_groups/item[N]`. Other FX families
/// remain owned by their loaders/schedulers and may consume the shared RNG
/// before environment effects are emitted. This does not start a whole group.
pub fn effect_group(fields: &Fields) -> Result<EffectGroupAsset, String> {
    let mut group = EffectGroupAsset::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => group.name = atom(value)?.to_owned(),
            "m_enabled" => group.enabled = boolean(value)?,
            "m_conditions" => {
                for (name, value) in scope(value)? {
                    if name != "m_location" {
                        return Err(format!("unknown FX group condition {name}"));
                    }
                    group.location = match atom(value)? {
                        "EVERYWHERE" | "0" => EffectGroupLocation::Everywhere,
                        "OUT_OF_WATER" | "1" => EffectGroupLocation::OutOfWater,
                        "UNDERWATER" | "2" => EffectGroupLocation::Underwater,
                        other => return Err(format!("unknown FX group location {other}")),
                    };
                }
            }
            "m_timings" => {
                for (name, value) in scope(value)? {
                    let range = match name.as_str() {
                        "m_delay" => &mut group.timings.delay,
                        "m_fadein" => &mut group.timings.fade_in,
                        "m_duration" => &mut group.timings.duration,
                        "m_fadeout" => &mut group.timings.fade_out,
                        _ => return Err(format!("unknown FX group timing {name}")),
                    };
                    bounds(range, scope(value)?)?;
                }
            }
            "m_envOverrides" => {
                descriptor_list(&mut group.environments, scope(value)?, environment_effect)?;
            }
            "m_sounds" => descriptor_list(&mut group.sounds, scope(value)?, sound_effect)?,
            "m_subFXs" => descriptor_list(&mut group.sub_effects, scope(value)?, sub_effect)?,
            "m_entityDefs" => descriptor_list(&mut group.entity_defs, scope(value)?, entity_def::entity_def)?,
            "m_explosions" => descriptor_list(&mut group.explosions, scope(value)?, explosion::explosion)?,
            "m_cameraShakes" => descriptor_list(&mut group.camera_shakes, scope(value)?, |fields| camera_shake(fields).map(std::sync::Arc::new))?,
            "m_screenFades" => screen_fade::list(&mut group.screen_fades, scope(value)?)?,
            "m_skinDecals" => descriptor_list(&mut group.skin_decals, scope(value)?, |fields| skin_decal(fields).map(std::sync::Arc::new))?,
            "m_frameCaptures" => descriptor_list(&mut group.frame_captures, scope(value)?, |fields| {
                let mut timings = Default::default();
                for (name, value) in fields {
                    match name.as_str() {
                        "m_name" | "m_enabled" => {}
                        "m_timings" => timings = explosion::standard_timings(scope(value)?)?,
                        _ => return Err(format!("unknown FX frame capture field {name}")),
                    }
                }
                Ok(timings)
            })?,
            _ => {} // Other effect families are not environment descriptors.
        }
    }
    Ok(group)
}

pub use effect_group as environment_group;

#[cfg(test)]
#[path = "fx/tests.rs"]
mod tests;
