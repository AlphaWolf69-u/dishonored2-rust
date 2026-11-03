//! Resolve each decal variant through the same cache and image-streaming owner
//! as ordinary WORLD/model materials.
use super::MaterialResolver;

impl MaterialResolver<'_> {
    /// Clone only the host material binding for direct projection submission.
    /// Parsed controls, texture resources and receiver geometry stay shared.
    pub(in crate::app) fn projected_decal_material(
        &mut self,
        name: &str,
        world: bool,
    ) -> Result<
        (
            super::super::MaterialHandle,
            Vec<(usize, crate::surface_gpu::ImageRole)>,
        ),
        String,
    > {
        let resolved = if world {
            self.get_world(name, 0, [0.; 3])
        } else {
            self.get(name, 0, [0.; 3])
        }
        .ok_or_else(|| format!("projected decal material not admitted: {name}"))?;
        let super::super::MaterialHandle::Decal(handle) = resolved.handle else {
            return Err(format!(
                "projected material does not use a decal program: {name}"
            ));
        };
        let mut material = self
            .decal_materials
            .get(&handle)
            .cloned()
            .ok_or("projected decal binding is absent")?;
        let program = material
            .extension
            .projection
            .as_ref()
            .ok_or("decal program controls absent")?
            .clone();
        let mut images = self
            .cache
            .get_for(
                name,
                if world {
                    super::MaterialGeometry::World
                } else {
                    super::MaterialGeometry::Model
                },
            )
            .and_then(Option::as_ref)
            .ok_or("decal image references absent")?
            .images
            .clone();
        if let Some(opacity) = program.opacity.as_deref() {
            let slot = self
                .register_image(opacity)
                .ok_or_else(|| format!("decal opacity image absent: {opacity}"))?;
            images.push((slot, crate::surface_gpu::ImageRole(39)));
        }
        material.extension.projected = true;
        Ok((
            super::super::MaterialHandle::Decal(self.decal_materials.add(material)),
            images,
        ))
    }
}
