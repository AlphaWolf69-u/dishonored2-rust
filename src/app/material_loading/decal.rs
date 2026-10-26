//! Resolve each decal variant through the same cache and image-streaming owner
//! as ordinary WORLD/model materials.
use super::{MaterialResolver, ResolvedMaterial};
use crate::assets::decal::DecalMaterials;

impl MaterialResolver<'_> {
    pub(in crate::app) fn decal_materials(
        &mut self,
        name: &str,
        position: [f32; 3],
    ) -> Result<DecalMaterials<ResolvedMaterial>, String> {
        let catalog = self.catalog.ok_or("decal material catalog unavailable")?;
        let references = DecalMaterials::read(catalog, name)?
            .ok_or_else(|| format!("decal declaration absent: {name}"))?
            .resolve(catalog)?;
        let mut load = |name: String, world: bool| {
            let resolved = if world {
                self.get_world(&name, 0, position)
            } else {
                self.get(&name, 0, position)
            };
            resolved.ok_or_else(|| format!("decal material not admitted: {name}"))
        };
        Ok(DecalMaterials {
            opaque: references
                .opaque
                .map(|name| load(name, false))
                .transpose()?,
            opaque_instanced: references
                .opaque_instanced
                .map(|name| load(name, true))
                .transpose()?,
            alpha: references.alpha.map(|name| load(name, false)).transpose()?,
            alpha_instanced: references
                .alpha_instanced
                .map(|name| load(name, true))
                .transpose()?,
        })
    }
}
