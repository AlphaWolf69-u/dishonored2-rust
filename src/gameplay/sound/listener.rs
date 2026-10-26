//! Audio-listener declarations and joint selection (14070a130/14070a7b0).
use crate::entities::{scene_motion::atom, Value};
use bevy_math::{Mat3, Vec3};
use super::{environment::Levels, spatial::Listener};

#[derive(Clone, Debug)]
pub struct Settings {
    pub index: i32,
    pub minimum: Levels,
    pub auto_registration: bool,
    pub override_spatialization: bool,
    pub swap_back_to_front: bool,
    pub collects_environment: bool,
    pub main_joint: String,
    pub peeking_joint: String,
}
impl Default for Settings {
    fn default() -> Self {
        // 14070a130: flag bits 0/3 set, bits 1/2 cleared; both levels zero.
        Self { index: 0, minimum: Levels::default(), auto_registration: true,
            override_spatialization: false, swap_back_to_front: false, collects_environment: true,
            main_joint: "camera_jnt".into(), peeking_joint: "center_root_jnt".into() }
    }
}
impl Settings {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let mut settings = Self::default();
        let boolean = |name, default| match atom(fields, name) {
            None => Ok(default), Some("true") => Ok(true), Some("false") => Ok(false),
            _ => Err(format!("invalid listener {name}")),
        };
        let number = |name| -> Result<f32, String> {
            let value = atom(fields, name).map(|value| value.parse::<f32>()).transpose()
                .map_err(|error| format!("listener {name}: {error}"))?.unwrap_or(0.0);
            if !value.is_finite() { return Err(format!("non-finite listener {name}")); }
            Ok(value)
        };
        if let Some(value) = atom(fields, "m_listenerIndex") {
            settings.index = value.parse().map_err(|error| format!("listener index: {error}"))?;
        }
        settings.minimum = Levels { obstruction: number("m_minimumObstruction")?, occlusion: number("m_minimumOcclusion")? };
        settings.auto_registration = boolean("m_autoSoundRegistration", true)?;
        settings.override_spatialization = boolean("m_overrideSpatialization", false)?;
        settings.swap_back_to_front = boolean("m_swapBackToFront", false)?;
        settings.collects_environment = boolean("m_collectsEnvironment", true)?;
        if let Some(value) = atom(fields, "m_mainjointName") { settings.main_joint = value.to_owned(); }
        if let Some(value) = atom(fields, "m_peekingJointName") { settings.peeking_joint = value.to_owned(); }
        Ok(settings)
    }
}

#[derive(Clone, Debug)]
pub struct Binding {
    pub settings: Settings,
    pub camera_joint: Option<usize>,
    pub main_joint: Option<usize>,
    pub peeking_joint: Option<usize>,
}
impl Binding {
    /// 14070a5d0 resolves camera_jnt independently from the position joints.
    /// Failed/empty position lookups retain the entity-frame branch.
    pub fn resolve(settings: Settings, names: &[String]) -> Self {
        let find = |name: &str| if name.is_empty() { None } else { names.iter().position(|candidate| candidate == name) };
        let main_joint = find(&settings.main_joint);
        let peeking_joint = find(&settings.peeking_joint);
        Self { camera_joint: find("camera_jnt"), settings, main_joint, peeking_joint }
    }

    pub fn transform(&self, origin: Vec3, axis: Mat3, peeking: bool,
        mut joint: impl FnMut(usize) -> Option<(Vec3, Mat3)>) -> Option<Listener>
    {
        let selected = if peeking && self.peeking_joint.is_some() { self.peeking_joint } else { self.main_joint };
        let (position, orientation) = if let Some(index) = selected {
            let position = joint(index)?.0;
            let orientation = match self.camera_joint { Some(index) => joint(index)?.1, None => Mat3::IDENTITY };
            (position, orientation)
        } else { (origin, axis) };
        let front = orientation.x_axis;
        let up = orientation.z_axis;
        Some(Listener { position, axis: Mat3::from_cols(front.cross(up), up, front), distance_scale: 1.0 })
    }
}
