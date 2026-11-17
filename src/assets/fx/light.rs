//!224340/2244c0 typed FX-light declaration; metadata1415054b0.
//! Curves/timings are partial edits of their actual native constructors.
use super::*;
use crate::gameplay::effects::{
    envelope::{ScalarEnvelope, VectorEnvelope},
    timing::EffectTimingRanges,
    vector_curve::{EffectVectorCurve, EffectVectorCurvePair},
};

pub struct LightPosition {
    pub track_position: bool,
    pub track_orientation: bool,
    pub offset: VectorEnvelope,
    pub worldspace: bool,
    pub randomize_each_loop: bool,
    pub culling_offset: [f32; 3],
}
pub struct LightAsset {
    pub name: String,
    pub enabled: bool,
    ///141a33810: point0, half-point1, spot2, area3, parallel4.
    pub kind: u32,
    pub preset: u32,
    pub renders_in_fog: bool,
    pub cast_shadows: bool,
    pub timing: EffectTimingRanges,
    pub looping: bool,
    pub finish_loop_on_stop: bool,
    /// decay, falloff, cutoff, angular-falloff, angular-range, back-reflection,
    /// specular attenuation: actual serialized2243d0 order.
    pub attenuation: [ScalarEnvelope; 7],
    pub randomize_attenuation: bool,
    /// value, origin-offset, scaleX, scaleY, offset, turbulence.
    pub scattering: [ScalarEnvelope; 6],
    pub randomize_scattering: bool,
    pub second_turbulence: bool,
    pub scatter_only: bool,
    pub volumetric_shadows: bool,
    pub preserve_volumetric_shadow: bool,
    pub custom_scatter_shape: bool,
    pub position: LightPosition,
    pub colour: VectorEnvelope,
    pub base_colour: [f32; 4],
    pub randomize_colour: bool,
}

fn colour_phase(start: [f32; 3], end: [f32; 3]) -> EffectVectorCurvePair<3> {
    //229fc0 ->00d760 mode4 stores cubic incoming/value/outgoing triples.
    EffectVectorCurvePair {
        maximum: EffectVectorCurve { times: vec![0., 1.],
            values: vec![[0.; 3], start, [0.; 3], [0.; 3], end, [0.; 3]] },
        minimum: EffectVectorCurve::default(),
    }
}
impl Default for LightAsset {
    fn default() -> Self {
        //2244c0; constants read from installed PE141abf158/154,
        //141ac8ff4/141ab3b04 and14228a8e0/142294e50.
        Self {
            name: String::new(), enabled: true, kind: 0, preset: 6,
            renders_in_fog: false, cast_shadows: true,
            timing: EffectTimingRanges::default(), looping: false, finish_loop_on_stop: false,
            attenuation: [1., 10., 5., 80., 10., 0., 0.].map(ScalarEnvelope::constant),
            randomize_attenuation: false,
            scattering: [0., 0., f32::from_bits(0x3a83126f), f32::from_bits(0x3a83126f), 0., 0.]
                .map(ScalarEnvelope::constant),
            randomize_scattering: false, second_turbulence: false, scatter_only: false,
            volumetric_shadows: false, preserve_volumetric_shadow: false, custom_scatter_shape: false,
            position: LightPosition { track_position: true, track_orientation: true,
                offset: VectorEnvelope::constant([0.; 3]), worldspace: false,
                randomize_each_loop: false, culling_offset: [0.; 3] },
            colour: VectorEnvelope { range: [[1.; 3]; 2],
                duration: EffectVectorCurvePair::constant([1.; 3]),
                fade_in: colour_phase([0.; 3], [1.; 3]),
                fade_out: colour_phase([1.; 3], [0.; 3]), uniform: false },
            base_colour: [1.; 4], randomize_colour: false,
        }
    }
}

pub fn light(fields: &[(String, Value)]) -> Result<LightAsset, String> {
    let mut result = LightAsset::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => result.name = atom(value)?.to_owned(),
            "m_enabled" => result.enabled = boolean(value)?,
            //224300 standard timings; reuse the same bounds reader without
            //constructing an unrelated temporary environment/curve resource.
            "m_timings" => for (name, value) in scope(value)? {
                match name.as_str() {
                    "m_looping" => result.looping = boolean(value)?,
                    "m_finishLoopOnStop" => result.finish_loop_on_stop = boolean(value)?,
                    "m_delay" => bounds(&mut result.timing.delay, scope(value)?)?,
                    "m_fadein" => bounds(&mut result.timing.fade_in, scope(value)?)?,
                    "m_duration" => bounds(&mut result.timing.duration, scope(value)?)?,
                    "m_fadeout" => bounds(&mut result.timing.fade_out, scope(value)?)?,
                    name => return Err(format!("unknown FX light timing field {name}")),
                }
            },
            "m_type" => result.kind = match atom(value)? {
                "LIGHT_POINT" => 0, "LIGHT_HALF_POINT" => 1, "LIGHT_SPOT" => 2,
                "LIGHT_AREA" => 3, "LIGHT_PARALLEL" => 4,
                name => return Err(format!("unknown FX light type {name}")),
            },
            "m_preset" => result.preset = crate::rendering::render_light::preset_index(Some(atom(value)?))
                .ok_or("unknown FX light preset")?,
            "m_rendersInFog" => result.renders_in_fog = boolean(value)?,
            "m_castShadows" => result.cast_shadows = boolean(value)?,
            "m_attenuation" => for (name, value) in scope(value)? {
                if name == "m_randomizeEachLoop" { result.randomize_attenuation = boolean(value)?; continue; }
                let index = match name.as_str() {
                    "m_decay" => 0, "m_fallOffDistance" => 1, "m_cutOffRange" => 2,
                    "m_angularFallOff" => 3, "m_angularRange" => 4,
                    "m_backReflection" => 5, "m_specularAttn" => 6,
                    name => return Err(format!("unknown FX light attenuation {name}")),
                };
                for (name, value) in scope(value)? {
                    if !particle_system::scalar(&mut result.attenuation[index], name, value)? {
                        return Err(format!("unknown FX light scalar field {name}"));
                    }
                }
            },
            "m_scattering" => for (name, value) in scope(value)? {
                let index = match name.as_str() {
                    "m_value" => Some(0), "m_originOffset" => Some(1),
                    "m_scatterShapeScaleX" => Some(2), "m_scatterShapeScaleY" => Some(3),
                    "m_scatterOffset" => Some(4), "m_scatterTurbulence" => Some(5),
                    "m_scatter2ndTurbulence" => { result.second_turbulence = boolean(value)?; None },
                    "m_scatterOnly" => { result.scatter_only = boolean(value)?; None },
                    "m_castVolumetricShadows" => { result.volumetric_shadows = boolean(value)?; None },
                    "m_dontAlterVolumetricDynaShadow" => { result.preserve_volumetric_shadow = boolean(value)?; None },
                    "m_scatterCustomShape" => { result.custom_scatter_shape = boolean(value)?; None },
                    "m_randomizeEachLoop" => { result.randomize_scattering = boolean(value)?; None },
                    name => return Err(format!("unknown FX light scattering {name}")),
                };
                if let Some(index) = index { for (name, value) in scope(value)? {
                    if !particle_system::scalar(&mut result.scattering[index], name, value)? {
                        return Err(format!("unknown FX light scalar field {name}"));
                    }
                } }
            },
            "m_position" => for (name, value) in scope(value)? {
                match name.as_str() {
                    "m_trackOwnerPosition" => result.position.track_position = boolean(value)?,
                    "m_trackOwnerOrientation" => result.position.track_orientation = boolean(value)?,
                    "m_offset" => for (name, value) in scope(value)? {
                        match name.as_str() {
                            "m_worldspace" => result.position.worldspace = boolean(value)?,
                            "m_randomizeEachLoop" => result.position.randomize_each_loop = boolean(value)?,
                            "m_cullingOffset" => particles::vector(&mut result.position.culling_offset, scope(value)?)?,
                            _ if particle_system::vector(&mut result.position.offset, name, value)? => {},
                            name => return Err(format!("unknown FX light offset field {name}")),
                        }
                    },
                    name => return Err(format!("unknown FX light position field {name}")),
                }
            },
            "m_color" => for (name, value) in scope(value)? {
                match name.as_str() {
                    "m_baseColor" => for (name, value) in scope(value)? {
                        let channel = match name.as_str() { "r" => 0, "g" => 1, "b" => 2, "a" => 3,
                            name => return Err(format!("unknown FX base-colour channel {name}")) };
                        result.base_colour[channel] = number(value)?;
                    },
                    "m_randomizeEachLoop" => result.randomize_colour = boolean(value)?,
                    _ if particle_system::vector(&mut result.colour, name, value)? => {},
                    name => return Err(format!("unknown FX light colour field {name}")),
                }
            },
            name => return Err(format!("unknown FX light field {name}")),
        }
    }
    Ok(result)
}
