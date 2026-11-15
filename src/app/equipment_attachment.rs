//! Child of PlayerArms: the selected ammo entity's rigid model and real weapon
//! tag binding (44dd00/62b240 ->875e80). No independent material/model owner.
use super::*;
use crate::{
    assets::equipment::{Descriptor as Weapon, attachment::Descriptor},
    gameplay::{entity_lifetime::EntityId, reference_frame::ReferenceFrame},
};
use std::{collections::HashMap, sync::Arc};

pub(in crate::app) struct PreparedModel {
    descriptor: Arc<Descriptor>,
    surfaces: Vec<ArmSurface>,
}

impl PreparedModel {
    pub(in crate::app) fn descriptor(&self) -> &Arc<Descriptor> {
        &self.descriptor
    }
}

pub(in crate::app) fn prepare(
    descriptor: Arc<Descriptor>,
    player_depth_hack: f32,
    resolver: &mut MaterialResolver,
    meshes: &mut dyn AssetWrite<Mesh>,
    commands: &mut super::super::setup_commands::SetupCommands,
) -> Result<PreparedModel, String> {
    let mut surfaces = Vec::new();
    for (index, surface) in descriptor.geometry.surfaces.iter().enumerate() {
        if let Some(surface) = prepare_rigid_surface(
            surface,
            index,
            descriptor
                .placement
                .custom_materials
                .get(index)
                .and_then(Option::as_deref),
            player_depth_hack,
            descriptor.cast_shadows,
            resolver,
            meshes,
            commands,
        )? {
            surfaces.push(surface);
        }
    }
    if surfaces.is_empty() {
        return Err(format!(
            "{}: attachment has no drawable surfaces",
            descriptor.entity
        ));
    }
    Ok(PreparedModel {
        descriptor,
        surfaces,
    })
}

struct Model {
    prepared: PreparedModel,
    // Source weapon rig is the actual tag owner, retained while bound.
    weapon: Arc<Weapon>,
    joint: usize,
    offset: Vec3,
    rotation: Mat3,
    visible: bool,
    shown: bool,
    frame: Option<ReferenceFrame>,
    bounds: Option<[[f32; 3]; 2]>,
    lighting: crate::probe_lighting::ProbeQuery,
}

#[derive(Default)]
pub(in crate::app) struct Models {
    live: HashMap<EntityId, Model>,
}
impl Models {
    /// The caller supplies visibility from the actual magazine callback. There
    /// is no inferred loaded round or fallback bind joint.
    pub(in crate::app) fn publish(
        &mut self,
        id: EntityId,
        weapon: Arc<Weapon>,
        tag: &str,
        prepared: PreparedModel,
        visible: bool,
    ) -> Result<(), String> {
        if self.live.contains_key(&id) {
            return Err("ammunition model generation already published".into());
        }
        let (joint, offset, rotation) = weapon.rig.tag(tag)?;
        self.live.insert(
            id,
            Model {
                prepared,
                weapon,
                joint,
                offset,
                rotation,
                visible,
                shown: false,
                frame: None,
                bounds: None,
                lighting: Default::default(),
            },
        );
        Ok(())
    }

    pub(in crate::app) fn set_visible(
        &mut self,
        id: EntityId,
        visible: bool,
    ) -> Result<(), String> {
        self.live
            .get_mut(&id)
            .ok_or("ammunition visibility targeted an absent generation")?
            .visible = visible;
        Ok(())
    }
    /// Native62b440 asks for the real attachment and its own hidden flag;
    /// frustum/view omission must not disable firing.
    pub(in crate::app) fn visible(&self, id: EntityId) -> bool {
        self.live.get(&id).is_some_and(|model| model.visible)
    }
    pub(in crate::app) fn frame(&self, id: EntityId) -> Option<ReferenceFrame> {
        self.live.get(&id).and_then(|model| model.frame)
    }
    pub(in crate::app) fn has(&self, id: EntityId) -> bool {
        self.live.contains_key(&id)
    }

    pub(in crate::app) fn update(
        &mut self,
        renders: &[ArmRender],
        visibility: &mut Query<&mut Visibility>,
        transforms: &mut Query<
            (&mut Transform, &mut GlobalTransform),
            (With<Mesh3d>, Without<Camera3d>),
        >,
        probes: Option<&crate::probe_lighting::ProbeLightingSet>,
        mut lighting: Option<&mut model_motion::ModelProbeLighting>,
        patches: &mut crate::rendering::bevy_backend::shader_buffer_patches::ShaderBufferPatches,
        buffers: &mut Assets<ShaderBuffer>,
    ) -> Result<(), String> {
        for model in self.live.values_mut() {
            let render = renders
                .iter()
                .find(|render| render.model == model.weapon.model && render.visible);
            let frame = render
                .map(|render| -> Result<ReferenceFrame, String> {
                    let rows = render
                        .joints
                        .get(model.joint)
                        .ok_or("weapon attachment tag joint is outside its actual pose")?;
                    let position = Vec3::new(rows[0][3], rows[1][3], rows[2][3]);
                    let rotation = Mat3::from_cols_array_2d(&std::array::from_fn(|column| {
                        std::array::from_fn(|row| rows[row][column])
                    }));
                    let position = position + rotation * model.offset;
                    Ok(ReferenceFrame {
                        position: position.to_array(),
                        basis: (rotation * model.rotation).to_cols_array_2d(),
                    })
                })
                .transpose()?;
            model.frame = frame;
            let shown = model.visible && frame.is_some();
            if let Some(frame) = frame {
                let placement = &model.prepared.descriptor.placement;
                let bounds = render_model::transformed_bounds(
                    model.prepared.descriptor.geometry.authored_bounds,
                    frame.position,
                    frame.basis,
                    placement.scale,
                );
                model.bounds = Some(bounds);
                publish_model_lighting(
                    bounds,
                    &mut model.lighting,
                    &model.prepared.surfaces,
                    probes,
                    lighting.as_deref_mut(),
                    patches,
                    buffers,
                );
                // Use the same coordinate/scale conversion as placed rigid
                // models, with this actual weapon-tag world frame.
                let rows = render_model::affine_rows(frame.position, frame.basis, placement.scale);
                let matrix = std::array::from_fn(|index| rows[index / 4][index % 4]);
                let transform = super::super::view_geometry::game_affine_to_view(&matrix);
                let global = GlobalTransform::from(transform);
                for part in &model.prepared.surfaces {
                    let (mut local, mut published) = transforms
                        .get_mut(part.entity)
                        .map_err(|_| "ammunition surface transform is absent")?;
                    *local = transform;
                    *published = global;
                }
            } else {
                model.bounds = None;
            }
            if model.shown != shown {
                for part in &model.prepared.surfaces {
                    *visibility
                        .get_mut(part.entity)
                        .map_err(|_| "ammunition surface visibility is absent")? = if shown {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    };
                }
                model.shown = shown;
            }
        }
        Ok(())
    }

    /// Despawn real draw entities, then transfer every source/geometry/material
    /// lease to PlayerArms' existing submitted-work acknowledgement.
    pub(in crate::app) fn remove(
        &mut self,
        id: EntityId,
        commands: &mut super::super::setup_commands::SetupCommands,
    ) -> Option<equipment::RetiredModel> {
        let model = self.live.remove(&id)?;
        let entities: Vec<_> = model
            .prepared
            .surfaces
            .iter()
            .map(|surface| surface.entity)
            .collect();
        commands.queue(move |world: &mut World| {
            for entity in entities {
                if world.entities().contains(entity) {
                    world.despawn(entity);
                }
            }
        });
        Some(equipment::RetiredModel::new(model))
    }

    pub(in crate::app) fn shown_materials(&self) -> impl Iterator<Item = &MaterialHandle> {
        self.live
            .values()
            .filter(|model| model.shown)
            .flat_map(|model| {
                model
                    .prepared
                    .surfaces
                    .iter()
                    .map(|surface| &surface.material)
            })
    }
    pub(in crate::app) fn submit_texture_demands(
        &self,
        state: Option<&crate::visibility::tome::RenderQueryState>,
        bindings: &bevy::platform::collections::HashMap<MaterialHandle, Vec<usize>>,
        demands: &[crate::texture::ProjectedAreaDemand],
    ) -> Result<(), String> {
        for model in self.live.values().filter(|model| model.shown) {
            let bounds = model
                .bounds
                .ok_or("shown ammunition has no published bounds")?;
            let (_, area) =
                crate::visibility::tome::render_query_aabb_visibility_without_depth(state, bounds)?;
            for part in &model.prepared.surfaces {
                for &slot in bindings.get(&part.material).into_iter().flatten() {
                    demands
                        .get(slot)
                        .ok_or("ammunition material references an unknown image")?
                        .submit_dynamic(area.unwrap_or(1.0));
                }
            }
        }
        Ok(())
    }
}
