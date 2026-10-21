//! Area fog loads the render model, independently of trigger containment hulls.
use crate::{
    assets::{
        archive::Catalog,
        entities::{self, Document},
        environment::ParameterReader,
    },
    rendering::fog::{alpha::VolumeFog, volume::VolumeSettings},
};
use bevy::prelude::*;

pub(super) struct AreaFogSource {
    placement: entities::PlacedModel,
    target: Option<Vec3>,
    settings: VolumeSettings,
    model: crate::bmodel::Model,
}

pub(super) fn load(catalog: &Catalog, document: &Document) -> Result<Vec<AreaFogSource>, String> {
    fn scope<'a>(
        fields: &'a [(String, entities::Value)],
        name: &str,
    ) -> Option<&'a [(String, entities::Value)]> {
        fields.iter().find_map(|(key, value)| {
            if key == name {
                if let entities::Value::Scope(fields) = value {
                    Some(fields.as_slice())
                } else {
                    None
                }
            } else {
                None
            }
        })
    }
    fn position(document: &Document, id: &str) -> Option<Vec3> {
        let definition = document
            .definitions
            .iter()
            .find(|d| d.kind == "entity" && d.id == id)?;
        let fields = scope(scope(&definition.fields, "edit")?, "spawnPosition")?;
        let mut xyz = [0.; 3];
        for (i, axis) in ["x", "y", "z"].into_iter().enumerate() {
            if let Some((_, entities::Value::Atom(value))) =
                fields.iter().find(|(key, _)| key == axis)
            {
                xyz[i] = value.trim_end_matches('m').parse().ok()?;
            }
        }
        Some(Vec3::from_array(xyz))
    }
    let mut result = Vec::new();
    for volume in entities::placed_environment_volumes(document) {
        let parameters = entities::environment_render_parameters_with_inheritance(
            &volume.id,
            &volume.environment_decl,
            |parent| {
                catalog
                    .read(&format!("generated/decls/env/{parent}.decl"))
                    .map(|bytes| bytes.map(|b| entities::declaration_text(&b).into_owned()))
                    .map_err(|e| e.to_string())
            },
        )?;
        let Some(settings) = VolumeSettings::resolve(&ParameterReader {
            catalog,
            parameters: &parameters,
        })?
        else {
            continue;
        };
        let model = volume
            .model
            .as_deref()
            .ok_or_else(|| format!("{}: fog volume has no render model", volume.id))?;
        let resource = if model.starts_with("maps/") {
            format!("{model}.bmodel")
        } else if model.ends_with(".lwo") {
            format!("generated/model/{}.bmodel", model.trim_end_matches(".lwo"))
        } else if model.ends_with(".bmodel") {
            format!("generated/model/{model}")
        } else {
            return Err(format!(
                "{}: unresolved fog render model {model}",
                volume.id
            ));
        };
        let bytes = catalog
            .read(&resource)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("{}: missing fog render model {resource}", volume.id))?;
        let model = crate::bmodel::parse(&bytes).map_err(|e| format!("{resource}: {e}"))?;
        result.push(AreaFogSource {
            target: volume
                .scattering_source
                .as_deref()
                .and_then(|id| position(document, id)),
            settings,
            placement: entities::PlacedModel {
                id: volume.id,
                resource_path: resource,
                position: volume.position,
                rotation: volume.rotation,
                scale: volume.scale,
                custom_materials: Vec::new(),
                max_visible_range: volume.max_visible_range,
            },
            model,
        });
    }
    Ok(result)
}

pub(super) struct AreaFog {
    meshes: Vec<Handle<Mesh>>,
    transform: Mat4,
    origin: Vec3,
    bounds: [[f32; 3]; 2],
    visible_range: f32,
    target: Option<Vec3>,
    settings: VolumeSettings,
}
impl AreaFogSource {
    pub(super) fn upload(self, meshes: &mut Assets<Mesh>) -> AreaFog {
        let transform = super::placed_transform(&self.placement).to_matrix();
        let bounds = crate::render_model::transformed_bounds(
            self.model.authored_bounds,
            self.placement.position,
            self.placement.rotation,
            self.placement.scale,
        );
        let mirrored = transform.determinant() < 0.;
        let handles = self
            .model
            .surfaces
            .into_iter()
            .map(|surface| {
                let mut mesh = Mesh::new(
                    bevy::mesh::PrimitiveTopology::TriangleList,
                    bevy::asset::RenderAssetUsages::RENDER_WORLD,
                );
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_POSITION,
                    surface
                        .positions
                        .into_iter()
                        .map(|p| super::game_to_view(p).to_array())
                        .collect::<Vec<_>>(),
                );
                mesh.insert_indices(super::view_geometry::render_indices(&surface.indices));
                meshes.add(super::view_geometry::mirrored_render_mesh(
                    mesh,
                    &surface.indices,
                    mirrored,
                ))
            })
            .collect();
        AreaFog {
            meshes: handles,
            transform,
            origin: Vec3::from_array(self.placement.position),
            bounds,
            visible_range: self.placement.max_visible_range.unwrap_or(0.),
            target: self.target,
            settings: self.settings,
        }
    }
}
impl AreaFog {
    pub(super) fn visible(
        &self,
        view: &AreaFogView,
        query: Option<&crate::tome::RenderAabbQuery>,
    ) -> Result<bool, String> {
        let model = crate::render_model::ModelQueryInput {
            // Prepared initial model state, as in initial_placed_model_query.
            // Shape flags cannot enable the bypass while frame bit1 is set.
            frame_flags: 1 | 4,
            shape_flags: 0,
            region: crate::render_model::range_region(self.bounds, self.visible_range),
            bounds: self.bounds,
        };
        let tome = query
            .map(|query| query.visibility_and_area(self.bounds))
            .transpose()?;
        Ok(crate::render_model::area_model_visible(
            model,
            view.eye,
            view.viewport,
            view.clip_rows,
            tome,
            Some(crate::render_model::ModelFadeInput {
                camera: view.eye,
                fov_radians: view.fov_radians,
                visible_range: self.visible_range,
                lod_scale: view.lod_scale,
            }),
        ))
    }

    pub(super) fn frame(
        &self,
        eye: Vec3,
        sun: Option<(Vec3, Vec3)>,
    ) -> Result<crate::rendering::bevy_backend::fog::AreaFogFrame, String> {
        let parameters: VolumeFog = self.settings.prepare(eye, self.origin, self.target, sun)?;
        Ok(crate::rendering::bevy_backend::fog::AreaFogFrame {
            meshes: self.meshes.clone(),
            world_from_local: self.transform,
            origin: self.origin,
            bounds: self.bounds,
            parameters,
        })
    }
}

pub(super) struct AreaFogView {
    pub eye: [f32; 3],
    pub viewport: [u32; 2],
    pub clip_rows: [[f32; 4]; 4],
    pub fov_radians: f32,
    pub lod_scale: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires installed level environments and area render models"]
    fn installed_area_fog_uses_render_geometry_and_resolved_settings() {
        let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        let mut levels = 0;
        let mut volumes = 0;
        let mut vertices = 0;
        for path in catalog.paths_with_suffix(".entities") {
            let bytes = catalog.read(&path).unwrap().unwrap();
            let document = entities::parse(&entities::declaration_text(&bytes)).unwrap();
            if entities::world_environment(&document).is_none() {
                continue;
            }
            levels += 1;
            for source in load(&catalog, &document).unwrap_or_else(|e| panic!("{path}: {e}")) {
                assert!(
                    !source.model.surfaces.is_empty(),
                    "{}",
                    source.placement.resource_path
                );
                vertices += source
                    .model
                    .surfaces
                    .iter()
                    .map(|s| s.positions.len())
                    .sum::<usize>();
                let mut meshes = Assets::<Mesh>::default();
                let fog = source.upload(&mut meshes);
                let frame = fog.frame(Vec3::ZERO, Some((Vec3::Z, Vec3::ONE))).unwrap();
                assert!(!frame.meshes.is_empty());
                assert!(frame.parameters.distance.is_finite());
                assert!(frame.bounds.iter().flatten().all(|value| value.is_finite()));
                for handle in &frame.meshes {
                    assert!(meshes.get(handle).is_some());
                }
                volumes += 1;
            }
        }
        assert_eq!((levels, volumes), (36, 157));
        assert!(vertices > 0);
        println!("Loaded {volumes} actual area-fog render models across {levels} levels, {vertices} vertices");
    }
}
