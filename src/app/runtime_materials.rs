//! Shared material-preparation results for runtime effects.
use bevy::prelude::*;
use std::sync::Arc;
use crate::rendering::particles::image_resources::ParticleImage;

pub(super) type RuntimeImages = Vec<(crate::surface_gpu::ImageRole, Arc<ParticleImage>)>;

#[derive(Clone)]
pub(super) struct PreparedDecal {
    pub name: String,
    pub world: bool,
    pub handle: Handle<super::DecalMaterial>,
    pub program: Arc<crate::rendering::decals::material::DecalProgram>,
    pub images: RuntimeImages,
}
