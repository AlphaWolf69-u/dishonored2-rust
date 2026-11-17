//! SoundVolumeControl7387d0/738810/738890/7389c0/738b30/738be0/738bf0.
//! The actual canonical acoustic zone is borrowed; no audio object or graph copy.
use super::acoustic_world::AcousticWorld;
use crate::{
    assets::entities::{Value, scene_motion::atom},
    gameplay::component_state::{ComponentInstance, ComponentOwner},
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct Definition {
    pub max_occlusion_strength: f32,
    pub max_ai_occlusion_strength: f32,
    pub default_ratio: f32,
}
impl Definition {
    ///738810 defaults and1416fb980 authored field offsets68/6c/70.
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let number = |key| -> Result<f32, String> {
            let value = atom(fields, key)
                .map(str::parse::<f32>)
                .transpose()
                .map_err(|error| format!("SoundVolumeControl {key}: {error}"))?
                .unwrap_or(1.);
            if !value.is_finite() {
                return Err(format!("nonfinite SoundVolumeControl {key}"));
            }
            Ok(value)
        };
        Ok(Self {
            max_occlusion_strength: number("m_maxOcclusionStrength")?,
            max_ai_occlusion_strength: number("m_maxAIOcclusionStrength")?,
            default_ratio: number("m_defaultOcclusionRatio")?,
        })
    }
}
pub struct Component {
    pub owner: ComponentOwner,
    pub definition: Option<Arc<Definition>>,
    pub state: State,
    pub started: bool,
}
impl Component {
    pub fn new(owner: ComponentOwner) -> Self {
        Self {
            owner,
            definition: None,
            state: State::default(),
            started: false,
        }
    }
}
impl ComponentInstance for Component {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}
#[derive(Clone, Copy, Debug)]
pub struct State {
    pub zone: Option<usize>,
    pub enabled: bool,
    pub strength: f32,
}
impl Default for State {
    fn default() -> Self {
        Self {
            zone: None,
            enabled: true,
            strength: 1.,
        }
    }
}
impl State {
    ///738bf0 uses the real render model's current world AABB midpoint.
    ///A successful outside-volume query retains None, as the source does.
    pub fn attach(
        &mut self,
        world: &mut AcousticWorld,
        model_bounds: [[f32; 3]; 2],
    ) -> Result<(), String> {
        self.zone = world.zone_at(std::array::from_fn(|i| {
            (model_bounds[0][i] + model_bounds[1][i]) * 0.5
        }))?;
        Ok(())
    }
    pub fn initialize(
        &mut self,
        definition: &Definition,
        world: &mut AcousticWorld,
        model_bounds: [[f32; 3]; 2],
        restored: bool,
    ) -> Result<(), String> {
        self.attach(world, model_bounds)?;
        let strength = if restored {
            self.strength
        } else {
            definition.default_ratio
        };
        self.set_strength(definition, world, strength, false)?;
        Ok(())
    }
    ///738890 does not clamp, and ignores setters while disabled. The return
    ///value requests the real owning entity's d4 bit4, never component flags.
    pub fn set_strength(
        &mut self,
        definition: &Definition,
        world: &mut AcousticWorld,
        strength: f32,
        mark_dirty: bool,
    ) -> Result<bool, String> {
        if !self.enabled {
            return Ok(false);
        }
        if let Some(zone) = self.zone {
            world.set_volume_strengths(
                zone,
                strength * definition.max_occlusion_strength,
                strength * definition.max_ai_occlusion_strength,
            )?;
        }
        self.strength = strength;
        Ok(mark_dirty)
    }
    ///7389c0: disabling zeros both strengths without discarding the retained
    ///ratio; enabling reapplies it and marks the actual entity dirty.
    pub fn set_enabled(
        &mut self,
        definition: &Definition,
        world: &mut AcousticWorld,
        enabled: bool,
    ) -> Result<bool, String> {
        let coefficients = if enabled {
            [
                self.strength * definition.max_occlusion_strength,
                self.strength * definition.max_ai_occlusion_strength,
            ]
        } else {
            [0.; 2]
        };
        if let Some(zone) = self.zone {
            world.set_volume_strengths(zone, coefficients[0], coefficients[1])?;
        }
        self.enabled = enabled;
        Ok(enabled)
    }
    ///738be0 clears the volume pointer, without restoring another writer's
    ///zone coefficients. Actual definition/common-owner release follows it.
    pub fn deinitialize(&mut self) {
        self.zone = None;
    }
}
