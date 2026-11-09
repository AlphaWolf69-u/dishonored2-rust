//! Supported sound-engine initialization inputs. The AudioMix session keeps
//! copied control IDs/ranges across maps; source syntax has no playback use.
use crate::archive::Catalog;
use super::RtpcDeclaration;

#[derive(Clone, Copy, Debug)]
pub struct EngineControls {
    pub volumes: [u32; 5],
    pub time_dilation: u32,
    pub physics_scale: RtpcDeclaration,
}
impl EngineControls {
    pub fn acquire(catalog: &Catalog) -> Result<Self, String> {
        let path = "generated/decls/soundengineinternals/default.soundengineinternals.decl";
        let source = crate::assets::declaration_texts::read_common_or_catalog(catalog, path)?
            .ok_or("sound engine internals declaration is absent")?;
        let fields = crate::entities::parse_declaration(&source)?;
        use crate::entities::scene_motion::{atom, child};
        let edit = child(&fields, "edit");
        let resolve = |field: &str| {
            let name = atom(edit, field).ok_or_else(|| format!("sound engine {field} is absent"))?;
            super::rtpc_resources::acquire_with(catalog, name,
                |path| crate::assets::declaration_texts::read_common_or_catalog(catalog, path))
        };
        let volume_fields = ["m_RTPC_MasterVolume", "m_RTPC_EffectsVolume", "m_RTPC_SpeechVolume",
            "m_RTPC_MusicVolume", "m_RTPC_VideoVolume"];
        let mut volumes = [0; 5];
        for (id, field) in volumes.iter_mut().zip(volume_fields) { *id = resolve(field)?.id; }
        Ok(Self { volumes, time_dilation: resolve("m_RTPC_TimeDilation")?.id,
            physics_scale: *resolve("m_RTPC_PhysicsScale")? })
    }
}
