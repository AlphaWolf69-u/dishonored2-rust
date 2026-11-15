//! The first-person arms, drawn the way the game draws them.
//!
//! The body component spawns one entity per arm (arkDeclCpntPlayerBody
//! m_leftArm / m_rightArm `m_entity`, FUN_140770560) and binds it to the
//! body. FUN_140771a00 marks the body and arm render models addAlways
//! (renderEntity +0x18bc bit 0x10: no frustum culling) and pushes
//! r_playerDepthHack, which FUN_1407724f0 set from the body decl's
//! m_depthHack, to the body; FUN_14087e0a0 hands it to the attached arms.
//! The arms use the world camera's projection: their vertex programs only
//! scale clip Z by the model's depth hack, and the depth prepass marks them
//! in stencil bit 0 for the "Depth buffer hack" unhack. castShadow is clear
//! (`m_castShadows = false`, runtime bit 3 clear), so they are not casters.
//! No first-person lighting path exists; they use the ordinary model
//! materials and probe lighting.
//!
//! Each arm surface is its own entity (the Tome-driven scene queue never
//! culls it). Its mesh holds the bind-pose vertices with all eight skin
//! weights; the GPU blends them (DH2_SKIN8, the installed skin VS order)
//! with the palette joint × inverse bind from `Player::arms()`, published
//! each frame as the model's joint transforms.
use super::view_geometry::{MeshUvs, source_mesh_from_uvs};
use super::{
    MaterialHandle, MaterialResolver, MeshTag, SceneData, ShaderBuffer, archive, bmd6,
    edge_skeleton, entities, material, model_motion, player::PlayerState, render_model,
    shadow_preparation, view_to_game,
};
use crate::gameplay::player::ArmRender;
use crate::rendering::bevy_backend::asset_preparation::AssetWrite;
use bevy::camera::CameraProjection;
use bevy::prelude::*;
use std::borrow::Cow;
#[path = "equipment_attachment.rs"]
pub(super) mod attachment;
#[path = "player_equipment.rs"]
mod equipment;

pub(super) use equipment::{RetiredModel, install_equipment_retirement};

struct ArmSurface {
    entity: Entity,
    // Retained through render extraction and GPU retirement.
    _mesh: Handle<Mesh>,
    source_index: usize,
    probe_index: u32,
    material: MaterialHandle,
}

struct ArmModel {
    managed_entity: Option<crate::gameplay::entity_lifetime::EntityId>,
    /// The md6 decl path, as `ArmRender::model` names it.
    model: String,
    /// One entity per palette entry whose transform is that entry; its
    /// surfaces' SkinnedMesh reads them (created on first use).
    joints: Vec<Entity>,
    inverse_bindposes: Option<Handle<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
    _equipment: Option<std::sync::Arc<crate::assets::equipment::Descriptor>>,
    skeleton: std::sync::Arc<edge_skeleton::EdgeSkeleton>,
    info: std::sync::Arc<bmd6::ModelInfo>,
    surfaces: Vec<ArmSurface>,
    lighting: crate::probe_lighting::ProbeQuery,
    shown: bool,
    /// Current world bounds (model vtable+88), while shown.
    world_bounds: Option<[[f32; 3]; 2]>,
    depth_hack: f32,
}

/// Retained through the end of an action so F8 need not catch a short pose.
/// Model indices use the same order as the individual diagnostic lines below.
#[derive(Clone, Copy, Debug, Default)]
struct RenderModelHistory {
    published_frames: u32,
    requested_frames: u32,
    skinned_frames: u32,
    skin_error_frames: u32,
    visible_surface_frames: u32,
    maximum_visible_surfaces: u32,
    swim_surface_visible_frames: u32,
    swim_underwater_visible_frames: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct RenderActionHistory {
    frames: u32,
    swim_surface_frames: u32,
    swim_underwater_frames: u32,
    swim_first: [Option<crate::gameplay::player::animation::SwimSnapshot>; 2],
    swim_last: [Option<crate::gameplay::player::animation::SwimSnapshot>; 2],
    swim_first_underwater: [Option<crate::gameplay::player::animation::SwimSnapshot>; 2],
    swim_first_moving: [Option<crate::gameplay::player::animation::SwimSnapshot>; 2],
    swim_phase_advanced: [bool; 2],
    swim_idle_advanced: [bool; 2],
    // The installed body declaration has at most two arms plus the body.
    models: [RenderModelHistory; 3],
}

#[derive(Resource)]
pub(super) struct PlayerArms {
    /// r_playerDepthHack after the body component's start.
    pub(super) player_depth_hack: f32,
    arms: Vec<ArmModel>,
    body_models: usize,
    pub(super) attachments: attachment::Models,
    retiring_equipment: Vec<equipment::RetiredModel>,
    /// `DH2_DEBUG_REST_ARMS=1`: draw the arm skeletons' rest pose 0.5 m in
    /// front of the camera instead of `Player::arms()`; another number sets
    /// that distance in metres (a diagnostic only).
    debug_rest: Option<f32>,
    action_active: [bool; 3],
    action_history: [Option<RenderActionHistory>; 3],
    /// Opt-in; capture holds at most 64 frames per action/model until the
    /// next matching action or level teardown.
    capture_geometry: bool,
    action_capture: [super::player_arm_capture::ActionCapture; 3],
}

pub(super) struct PreparedEquipmentModel(ArmModel);

impl PreparedEquipmentModel {
    pub(super) fn entity(&self) -> crate::gameplay::entity_lifetime::EntityId {
        self.0
            .managed_entity
            .expect("prepared equipment retains its managed generation")
    }
    pub(super) fn descriptor(&self) -> &std::sync::Arc<crate::assets::equipment::Descriptor> {
        self.0
            ._equipment
            .as_ref()
            .expect("prepared equipment retains its actual descriptor")
    }
}

impl PlayerArms {
    pub(super) fn retire_completed_models(&mut self) {
        self.retiring_equipment.retain(|entry| !entry.complete());
    }
}

pub(super) fn prepare_equipment_model(
    entity: crate::gameplay::entity_lifetime::EntityId,
    descriptor: std::sync::Arc<crate::assets::equipment::Descriptor>,
    player_depth_hack: f32,
    resolver: &mut MaterialResolver,
    meshes: &mut dyn AssetWrite<Mesh>,
    commands: &mut super::setup_commands::SetupCommands,
) -> Result<PreparedEquipmentModel, String> {
    let (hack, parameter) = arm_model_depth_hack(player_depth_hack);
    let mut model = prepare_model(
        descriptor.model.clone(),
        descriptor.geometry.clone(),
        descriptor.rig.skeleton.clone(),
        hack,
        parameter,
        &[],
        true,
        resolver,
        meshes,
        commands,
    )?;
    if model.surfaces.is_empty() {
        return Err(format!(
            "{} has no drawable equipment surfaces",
            descriptor.model
        ));
    }
    model.managed_entity = Some(entity);
    model._equipment = Some(descriptor);
    Ok(PreparedEquipmentModel(model))
}

/// Ordinary rigid BML1 surface admission for authored equipment children.
/// This is the same geometry/material owner as the skinned parent; it does
/// not invent a skeleton for an entity whose source model is rigid.
#[allow(clippy::too_many_arguments)]
fn prepare_rigid_surface(
    source: &crate::bmodel::Surface,
    source_index: usize,
    material_override: Option<&str>,
    player_depth_hack: f32,
    cast_shadows: bool,
    resolver: &mut MaterialResolver,
    meshes: &mut dyn AssetWrite<Mesh>,
    commands: &mut super::setup_commands::SetupCommands,
) -> Result<Option<ArmSurface>, String> {
    let path = material_override.unwrap_or(&source.material_path);
    if material::is_null_material(path) || source.positions.is_empty() || source.indices.is_empty()
    {
        return Ok(None);
    }
    let resolved = resolver
        .get(path, 0, [0.0; 3])
        .ok_or_else(|| format!("equipment child material unresolved: {path}"))?;
    let MaterialHandle::Probe(handle) = &resolved.handle else {
        return Err(format!(
            "equipment child material is not an ordinary model material: {path}"
        ));
    };
    let mut mesh = source_mesh_from_uvs(
        &source.positions,
        MeshUvs::Packed(&source.uvs),
        &source.indices,
        &source.packed_channels,
        false,
    );
    let (hack, parameter) = arm_model_depth_hack(player_depth_hack);
    if hack > 0.0 {
        mesh.insert_attribute(
            super::materials::DH2_MODEL_DEPTH_HACK,
            vec![parameter.unwrap_or(1.0); source.positions.len()],
        );
    }
    mesh.asset_usage = bevy::asset::RenderAssetUsages::RENDER_WORLD;
    let mesh = meshes.add(mesh);
    let mut entity = commands.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(handle.clone()),
        Transform::IDENTITY,
        GlobalTransform::IDENTITY,
        Visibility::Hidden,
        MeshTag(resolved.probe_index),
        bevy::camera::visibility::NoFrustumCulling,
    ));
    if !cast_shadows {
        entity.insert(bevy::light::NotShadowCaster);
    }
    Ok(Some(ArmSurface {
        entity: entity.id(),
        _mesh: mesh,
        source_index,
        probe_index: resolved.probe_index,
        material: resolved.handle.clone(),
    }))
}

fn publish_model_lighting(
    bounds: [[f32; 3]; 2],
    query: &mut crate::probe_lighting::ProbeQuery,
    surfaces: &[ArmSurface],
    probes: Option<&crate::probe_lighting::ProbeLightingSet>,
    buffer: Option<&mut model_motion::ModelProbeLighting>,
    patches: &mut crate::rendering::bevy_backend::shader_buffer_patches::ShaderBufferPatches,
    buffers: &mut Assets<ShaderBuffer>,
) {
    let center = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5);
    if let (Some(probes), Some(buffer)) = (probes, buffer) {
        if let Some(lighting) = probes.interpolate_query_sh(center, query) {
            let record = model_motion::probe_record_bytes(lighting.object_shader_record());
            for part in surfaces {
                if part.probe_index & 0x8000_0000 == 0 {
                    buffer.write(patches, buffers, part.probe_index, &record);
                }
            }
        }
    }
}

impl PlayerArms {
    pub(super) fn publish_equipment_model(
        &mut self,
        model: PreparedEquipmentModel,
    ) -> Result<(), String> {
        if self
            .arms
            .iter()
            .any(|entry| entry.managed_entity == model.0.managed_entity)
        {
            return Err(format!(
                "equipment model already published: {}",
                model.0.model
            ));
        }
        self.arms.push(model.0);
        Ok(())
    }

    pub(super) fn has_equipment_model(
        &self,
        entity: crate::gameplay::entity_lifetime::EntityId,
    ) -> bool {
        self.arms
            .iter()
            .skip(self.body_models)
            .any(|model| model.managed_entity == Some(entity))
    }

    pub(super) fn equipment_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.arms
            .iter()
            .skip(self.body_models)
            .flat_map(|model| model.surfaces.iter().map(|part| part.entity))
    }
    /// Remove draw/joint entities immediately; retain the resource leases
    /// until the render queue acknowledges their final submitted work.
    pub(super) fn remove_equipment_model(
        &mut self,
        model: crate::gameplay::entity_lifetime::EntityId,
        commands: &mut super::setup_commands::SetupCommands,
    ) -> Option<RetiredModel> {
        let Some(index) = self
            .arms
            .iter()
            .enumerate()
            .skip(self.body_models)
            .find_map(|(index, entry)| (entry.managed_entity == Some(model)).then_some(index))
        else {
            return None;
        };
        let entry = self.arms.remove(index);
        let entities: Vec<_> = entry
            .surfaces
            .iter()
            .map(|part| part.entity)
            .chain(entry.joints.iter().copied())
            .collect();
        commands.queue(move |world: &mut World| {
            for entity in entities {
                if world.entities().contains(entity) {
                    world.despawn(entity);
                }
            }
        });
        let receipt = equipment::RetiredModel::new(entry);
        self.retiring_equipment.push(receipt.clone());
        Some(receipt)
    }

    pub(super) fn remove_attachment_model(
        &mut self,
        id: crate::gameplay::entity_lifetime::EntityId,
        commands: &mut super::setup_commands::SetupCommands,
    ) -> Option<RetiredModel> {
        let receipt = self.attachments.remove(id, commands)?;
        self.retiring_equipment.push(receipt.clone());
        Some(receipt)
    }

    /// F8-only snapshot of the render owner's published arm state.
    pub(super) fn diagnostic(
        &self,
        probes: Option<&crate::probe_lighting::ProbeLightingSet>,
        published: Option<&model_motion::ModelProbeLighting>,
        buffers: &Assets<ShaderBuffer>,
    ) -> String {
        let mut report: String = self
            .arms
            .iter()
            .map(|arm| {
                format!(
                    "  render model={} shown={} surfaces={} joints={} bounds={:?}\n",
                    arm.model,
                    arm.shown,
                    arm.surfaces.len(),
                    arm.joints.len(),
                    arm.world_bounds
                )
            })
            .collect();
        // F8 only: replay the published model-bounds query, without changing
        // its cache or adding work to skinning/lighting on ordinary frames.
        for arm in &self.arms {
            let sample = arm
                .world_bounds
                .map(|bounds| std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5));
            let mut query = arm.lighting;
            let lighting = probes
                .zip(sample)
                .and_then(|(probes, sample)| probes.interpolate_query_sh(sample, &mut query));
            report.push_str(&format!("  lighting model={} depth_hack={} query={:?} sample={sample:?} resolved={lighting:?}\n", arm.model, arm.depth_hack, arm.lighting));
            for part in &arm.surfaces {
                let record = published
                    .and_then(|published| published.diagnostic_record(buffers, part.probe_index));
                report.push_str(&format!("    surface={} probe_index={} source_material={} published_cpu_record={record:?}\n", part.source_index, part.probe_index, arm.info.surfaces[part.source_index].material_path));
            }
        }
        report.push_str(&format!(
            "  renderer action history [mantle, catfall, swimming]={:?}\n",
            self.action_history
        ));
        if self.capture_geometry {
            for (action, capture) in ["mantle", "catfall", "swimming"]
                .into_iter()
                .zip(&self.action_capture)
            {
                for (index, arm) in self.arms.iter().enumerate() {
                    let surfaces: Vec<_> = arm
                        .surfaces
                        .iter()
                        .map(|surface| surface.source_index)
                        .collect();
                    report.push_str(&format!(
                        "  {action} geometry {}: {}\n",
                        arm.model,
                        capture.describe(index, &arm.info, &surfaces, arm.depth_hack)
                    ));
                }
            }
        } else {
            report.push_str(
                "  hand geometry history disabled (DH2_DEBUG_HAND_GEOMETRY=1 enables it)\n",
            );
        }
        report
    }
    /// Materials of the arms on screen, whose textures stream like those of
    /// visible scene instances.
    pub(super) fn shown_materials(&self) -> impl Iterator<Item = &MaterialHandle> {
        self.arms
            .iter()
            .filter(|arm| arm.shown)
            .flat_map(|arm| arm.surfaces.iter().map(|part| &part.material))
            .chain(self.attachments.shown_materials())
    }
}

impl PlayerArms {
    /// FUN_14031ad30 (dynamic model gather) gives every drawn model's
    /// material images its FUN_140a764c0 flag-0 area for the model's current
    /// world bounds (1.0 when the query leaves it untouched), through
    /// FUN_14031a510 -> FUN_14031a410's maximum. The arms are dynamic render
    /// models, so their images stream by their own projected area.
    pub(super) fn submit_texture_demands(
        &self,
        state: Option<&crate::visibility::tome::RenderQueryState>,
        bindings: &bevy::platform::collections::HashMap<MaterialHandle, Vec<usize>>,
        demands: &[crate::texture::ProjectedAreaDemand],
    ) -> Result<(), String> {
        for arm in self.arms.iter().filter(|arm| arm.shown) {
            let Some(bounds) = arm.world_bounds else {
                continue;
            };
            let (_, area) =
                crate::visibility::tome::render_query_aabb_visibility_without_depth(state, bounds)?;
            let area = area.unwrap_or(1.0);
            for part in &arm.surfaces {
                for &slot in bindings.get(&part.material).into_iter().flatten() {
                    demands
                        .get(slot)
                        .ok_or("arm material references an unknown image")?
                        .submit_dynamic(area);
                }
            }
        }
        self.attachments
            .submit_texture_demands(state, bindings, demands)
    }
}

/// The model depth hack each arm's render model resolves (FUN_14087e0a0):
/// the body entity receives r_playerDepthHack (FUN_14087b760), the arms
/// keep a zero local value and inherit it through the attachment. The body
/// model's flags at runtime carried only addAlways (castShadow clear).
fn arm_model_depth_hack(player_depth_hack: f32) -> (f32, Option<f32>) {
    let body = body_model_depth_state(player_depth_hack);
    let inherited =
        render_model::parent_model_depth_state(Some((0, Some((0x10, body.depth_hack)))));
    let arm = render_model::resolve_inherited_model_depth_state(inherited, 0.0, 0, 0);
    let (mut flags, mut depth_hack) = (render_model::INITIAL_MODEL_CONTEXT_FLAGS_18BC, 0.0);
    let parameter = render_model::publish_model_depth_state(&mut flags, &mut depth_hack, arm);
    (depth_hack, parameter)
}

/// The body entity's own model state: r_playerDepthHack set on it.
fn body_model_depth_state(player_depth_hack: f32) -> render_model::InheritedModelDepthState {
    render_model::resolve_inherited_model_depth_state(
        render_model::parent_model_depth_state(None),
        player_depth_hack,
        0,
        0,
    )
}

/// The body model's published depth hack and parameter.
fn body_model_depth_hack(player_depth_hack: f32) -> (f32, Option<f32>) {
    let body = body_model_depth_state(player_depth_hack);
    let (mut flags, mut depth_hack) = (render_model::INITIAL_MODEL_CONTEXT_FLAGS_18BC, 0.0);
    let parameter = render_model::publish_model_depth_state(&mut flags, &mut depth_hack, body);
    (depth_hack, parameter)
}

fn character_name(character: crate::gameplay::pause_state::Character) -> &'static str {
    match character {
        crate::gameplay::pause_state::Character::Corvo => "corvo",
        crate::gameplay::pause_state::Character::Emily => "emily",
    }
}

/// Load both arm render models with their real materials and spawn their
/// (hidden) surface entities.
pub(super) fn load_player_arms(
    catalog: &archive::Catalog,
    resolver: &mut MaterialResolver,
    meshes: &mut dyn AssetWrite<Mesh>,
    commands: &mut super::setup_commands::SetupCommands,
    declarations_source: &mut entities::player::DeclarationSource,
) -> Result<PlayerArms, String> {
    let text = |path: &str| -> Result<Option<String>, String> {
        Ok(catalog
            .read(path)
            .map_err(|e| e.to_string())?
            .map(entities::owned_declaration_text))
    };
    let character = character_name(super::pause_menu::debug_character());
    let declarations = declarations_source.full(catalog, character)?;
    let view = entities::player::player_view_models(&declarations.body, text)?;
    // The arm entities and the body entity's own model (drawn only while
    // the legs are visible).
    let arm_hack = arm_model_depth_hack(view.depth_hack);
    let models = view
        .arms
        .into_iter()
        .flatten()
        .map(|model| (model, arm_hack))
        .chain(std::iter::once((
            declarations.body_model.clone(),
            body_model_depth_hack(view.depth_hack),
        )));
    let mut arms = Vec::new();
    let mut model_sources = bmd6::ModelSources::new(catalog);
    for (model, (model_depth_hack, parameter)) in models {
        let stem = model
            .strip_suffix(".md6")
            .ok_or_else(|| format!("arm model {model} is not an md6"))?;
        let resource = format!("generated/basemodel/{stem}.bmd6model");
        let declaration = |path: &str| text(path).ok().flatten();
        let info = model_sources.acquire(catalog, &resource)?;
        let binding = bmd6::resolve_skeleton_binding(&resource, declaration)?;
        let skeleton_name = binding
            .as_ref()
            .and_then(|binding| binding.name.as_deref())
            .filter(|name| !name.is_empty())
            .or_else(|| (!info.skeleton_name.is_empty()).then_some(info.skeleton_name.as_str()))
            .ok_or_else(|| format!("{model} has no skeleton binding"))?;
        let skeleton = shadow_preparation::read_skeleton(catalog, skeleton_name)?;
        if skeleton.inverse_bind.len() < skeleton.rest.len() {
            return Err(format!(
                "{skeleton_name} has fewer inverse binds than joints"
            ));
        }
        let material_overrides = if model == declarations.body_model {
            view.visible_legs_materials.as_slice()
        } else {
            &[]
        };
        arms.push(prepare_model(
            model,
            info,
            skeleton,
            model_depth_hack,
            parameter,
            material_overrides,
            false,
            resolver,
            meshes,
            commands,
        )?);
    }
    let capture_geometry = std::env::var("DH2_DEBUG_HAND_GEOMETRY").as_deref() == Ok("1");
    crate::platform::client_diagnostics::record(&format!(
        "Hand action geometry recording enabled={capture_geometry}"
    ));
    Ok(PlayerArms {
        player_depth_hack: view.depth_hack,
        body_models: arms.len(),
        attachments: Default::default(),
        arms,
        retiring_equipment: Vec::new(),
        debug_rest: std::env::var("DH2_DEBUG_REST_ARMS").ok().and_then(|value| {
            if value == "1" {
                Some(0.5)
            } else {
                value.parse().ok()
            }
        }),
        action_active: [false; 3],
        action_history: [None; 3],
        capture_geometry,
        action_capture: Default::default(),
    })
}

/// The one first-person geometry/material/skin admission path. Equipment
/// supplies its actual rig/model acquisitions; it never creates another cache.
#[allow(clippy::too_many_arguments)]
fn prepare_model(
    model: String,
    info: std::sync::Arc<bmd6::ModelInfo>,
    skeleton: std::sync::Arc<edge_skeleton::EdgeSkeleton>,
    model_depth_hack: f32,
    parameter: Option<f32>,
    material_overrides: &[String],
    require_materials: bool,
    resolver: &mut MaterialResolver,
    meshes: &mut dyn AssetWrite<Mesh>,
    commands: &mut super::setup_commands::SetupCommands,
) -> Result<ArmModel, String> {
    if skeleton.inverse_bind.len() < skeleton.rest.len() || skeleton.rest.is_empty() {
        return Err(format!(
            "{model} has an invalid skeleton/inverse-bind pairing"
        ));
    }
    let rest = edge_skeleton::pose_matrices(
        &skeleton.rest,
        &skeleton.hierarchy,
        &edge_skeleton::JointRestPose {
            rotation: [0.0, 0.0, 0.0, 1.0],
            translation: [0.0; 3],
            scale: [1.0; 3],
        },
    )?;
    let palette = edge_skeleton::skin_palette(&rest, &skeleton.inverse_bind[..rest.len()])?;
    if palette.len() > bevy::pbr::MAX_JOINTS {
        return Err(format!(
            "{model} has {} joints, beyond a GPU skin's {}",
            palette.len(),
            bevy::pbr::MAX_JOINTS
        ));
    }
    let mut prepared = Vec::new();
    for (index, surface) in info.surfaces.iter().enumerate() {
        if material::is_null_material(&surface.material_path)
            || surface.positions.is_empty()
            || surface.indices.is_empty()
        {
            continue;
        }
        let material_path = material_overrides
            .get(index)
            .map(String::as_str)
            .unwrap_or(&surface.material_path);
        let Some(resolved) = resolver.get(material_path, 0, [0.0; 3]) else {
            if require_materials {
                return Err(format!(
                    "equipment material unresolved: {model} {material_path}"
                ));
            }
            eprintln!("Arm material unresolved {model} {material_path}");
            continue;
        };
        let MaterialHandle::Probe(handle) = &resolved.handle else {
            return Err(format!(
                "{model} surface {index} material {} is not a forward model material",
                surface.material_path
            ));
        };
        // The surface's vertex declaration fixes its weights per vertex
        // (decl 9: four, else eight), the stream the skinning reads.
        let influences = surface.skin_influences;
        let mut mesh = source_mesh_from_uvs(
            &surface.positions,
            MeshUvs::Packed(&surface.uvs),
            &surface.indices,
            &surface.packed_channels,
            false,
        );
        bind_pose_skin(&mut mesh, surface, influences, palette.len())
            .map_err(|error| format!("{model} surface {index}: {error}"))?;
        if model_depth_hack > 0.0 {
            // Non-unit positive values are published; otherwise the
            // programs read the view default 1.0 (FUN_1403586a0).
            let value = parameter.unwrap_or(1.0);
            mesh.insert_attribute(
                super::materials::DH2_MODEL_DEPTH_HACK,
                vec![value; surface.positions.len()],
            );
        }
        // Skinned on the GPU: the vertices never change after upload.
        mesh.asset_usage = bevy::asset::RenderAssetUsages::RENDER_WORLD;
        prepared.push((
            index,
            mesh,
            resolved.probe_index,
            resolved.handle.clone(),
            handle.clone(),
        ));
    }
    // Validate every surface before reserving/spawning anything. A failed
    // equipment admission cannot leave a half-created render entity.
    let mut surfaces = Vec::with_capacity(prepared.len());
    for (index, mesh, probe_index, material, handle) in prepared {
        let mesh = meshes.add(mesh);
        let entity = commands
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(handle),
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                Visibility::Hidden,
                MeshTag(probe_index),
                bevy::camera::visibility::NoFrustumCulling,
                bevy::light::NotShadowCaster,
            ))
            .id();
        surfaces.push(ArmSurface {
            entity,
            _mesh: mesh,
            source_index: index,
            probe_index,
            material,
        });
    }
    Ok(ArmModel {
        managed_entity: None,
        model,
        joints: Vec::new(),
        inverse_bindposes: None,
        _equipment: None,
        skeleton,
        info,
        surfaces,
        lighting: Default::default(),
        shown: false,
        world_bounds: None,
        depth_hack: if model_depth_hack > 0.0 {
            parameter.unwrap_or(1.0)
        } else {
            1.0
        },
    })
}

/// The rest pose with the arm entity frame placed in front of the camera,
/// oriented like the body's spine03 rest frame (local x left, y up, z
/// forward). Diagnostic geometry only.
fn debug_rest_arms(arms: &PlayerArms, camera: &Transform, distance: f32) -> Vec<ArmRender> {
    let direction = |v: Vec3| Vec3::from_array(view_to_game(v));
    let forward = direction(*camera.forward());
    let up = direction(*camera.up());
    let left = -direction(*camera.right());
    let origin =
        Vec3::from_array(view_to_game(camera.translation)) + forward * distance - up * 0.15;
    let rotation = Quat::from_mat3(&Mat3::from_cols(left, up, forward));
    let root = edge_skeleton::JointRestPose {
        rotation: rotation.to_array(),
        translation: origin.to_array(),
        scale: [1.0; 3],
    };
    arms.arms
        .iter()
        .take(arms.body_models)
        .filter_map(|arm| {
            let joints =
                edge_skeleton::pose_matrices(&arm.skeleton.rest, &arm.skeleton.hierarchy, &root)
                    .ok()?;
            Some(ArmRender {
                managed_entity: None,
                model: arm.model.clone(),
                joints,
                visible: true,
            })
        })
        .collect()
}

/// The installed skin stream in bind pose for GPU skinning: game-space
/// positions and authored N/T/B (the palette carries the axis conversion),
/// joints 0..3 in Bevy's joint attributes and 4..7 in DH2's, each joint
/// offset by the surface's palette start. A joint past the palette must
/// carry no weight (as `edge_skeleton::vertex_skin_matrix` requires); it is
/// then pointed at joint 0, contributing nothing.
fn bind_pose_skin(
    mesh: &mut Mesh,
    surface: &bmd6::SurfaceInfo,
    influences: u8,
    joints: usize,
) -> Result<(), String> {
    if !matches!(influences, 4 | 8) {
        return Err("skin input must have four or eight weights".into());
    }
    let count = surface.positions.len();
    if surface.skin_indices.len() != count
        || surface.skin_weights.len() != count
        || surface.packed_channels.len() != count
    {
        return Err("skin vertex streams have different lengths".into());
    }
    let snorm = |b: u8| ((b as i8 as f32) / 127.0).max(-1.0);
    let mut normals = Vec::with_capacity(count);
    let mut tangents = Vec::with_capacity(count);
    let mut bitangents = Vec::with_capacity(count);
    let mut first = Vec::with_capacity(count);
    let mut first_weights = Vec::with_capacity(count);
    let mut second = Vec::with_capacity(count);
    let mut second_weights = Vec::with_capacity(count);
    for i in 0..count {
        let packed = surface.packed_channels[i];
        let n = Vec3::new(snorm(packed[4]), snorm(packed[5]), snorm(packed[6]));
        let t = Vec3::new(snorm(packed[8]), snorm(packed[9]), snorm(packed[10]));
        let sign = snorm(packed[11]);
        normals.push(n.to_array());
        tangents.push(t.extend(sign).to_array());
        bitangents.push((n.cross(t) * sign).to_array());
        let mut index = [0u32; 8];
        let mut weight = [0.0f32; 8];
        for k in 0..usize::from(influences) {
            let joint = surface
                .matrix_offset
                .checked_add(u32::from(surface.skin_indices[i][k]))
                .ok_or("skin joint index overflow")?;
            let w = surface.skin_weights[i][k];
            if (joint as usize) < joints {
                index[k] = joint;
            } else if w != 0 {
                return Err(format!(
                    "skin joint {joint} exceeds {joints} actual palette entries"
                ));
            }
            weight[k] = f32::from(w) / 255.0;
        }
        first.push([
            index[0] as u16,
            index[1] as u16,
            index[2] as u16,
            index[3] as u16,
        ]);
        first_weights.push([weight[0], weight[1], weight[2], weight[3]]);
        second.push([index[4], index[5], index[6], index[7]]);
        second_weights.push([weight[4], weight[5], weight[6], weight[7]]);
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, surface.positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
    mesh.insert_attribute(super::materials::DH2_BITANGENT, bitangents);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_JOINT_INDEX,
        bevy::mesh::VertexAttributeValues::Uint16x4(first),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, first_weights);
    mesh.insert_attribute(
        super::materials::DH2_JOINT_INDEX_B,
        bevy::mesh::VertexAttributeValues::Uint32x4(second),
    );
    mesh.insert_attribute(super::materials::DH2_JOINT_WEIGHT_B, second_weights);
    Ok(())
}

/// A palette entry (game-space 3x4 rows) as a view-space joint transform:
/// game_to_view (x, z, -y) only reorders and negates rows, so folding it into
/// the palette is exact.
fn joint_transform(matrix: &[[f32; 4]; 3]) -> GlobalTransform {
    let rows = [matrix[0], matrix[2], matrix[1].map(|value| -value)];
    let column = |c: usize| bevy::math::Vec3A::new(rows[0][c], rows[1][c], rows[2][c]);
    GlobalTransform::from(bevy::math::Affine3A {
        matrix3: bevy::math::Mat3A::from_cols(column(0), column(1), column(2)),
        translation: column(3),
    })
}

/// One palette/joint publication path for the body, arms, equipment and
/// authored animated equipment children. The caller supplies the actual
/// entity tree's complete world joint pose, never a rigid substitute.
fn publish_model_skin(
    arm: &mut ArmModel,
    render: &ArmRender,
    commands: &mut Commands,
    inverse_bindposes: &mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
    joint_transforms: &mut Query<&mut GlobalTransform, Without<Mesh3d>>,
) -> Result<Vec<[[f32; 4]; 3]>, String> {
    let count = arm.skeleton.rest.len();
    if render.joints.len() != count {
        return Err(format!(
            "{} pose has {} joints, skeleton {count}",
            arm.model,
            render.joints.len()
        ));
    }
    let palette = edge_skeleton::skin_palette(&render.joints, &arm.skeleton.inverse_bind[..count])?;
    if arm.joints.is_empty() {
        // Bevy's joint matrices are joint transform x inverse bind;
        // the palette already includes the inverse bind.
        arm.joints = palette
            .iter()
            .map(|matrix| commands.spawn(joint_transform(matrix)).id())
            .collect();
        let identity =
            inverse_bindposes.add(bevy::mesh::skinning::SkinnedMeshInverseBindposes::from(
                vec![Mat4::IDENTITY; palette.len()],
            ));
        for part in &arm.surfaces {
            commands
                .entity(part.entity)
                .insert(bevy::mesh::skinning::SkinnedMesh {
                    inverse_bindposes: identity.clone(),
                    joints: arm.joints.clone(),
                });
        }
        arm.inverse_bindposes = Some(identity);
    } else {
        for (&joint, matrix) in arm.joints.iter().zip(&palette) {
            if let Ok(mut transform) = joint_transforms.get_mut(joint) {
                *transform = joint_transform(matrix);
            }
        }
    }
    Ok(palette)
}

/// Skin and show the arms `Player::arms()` names (left, then right); hide
/// arm models it omits or marks invisible.
pub(super) fn update_player_arms(
    state: Option<Res<PlayerState>>,
    arms: Option<ResMut<PlayerArms>>,
    cameras: Query<(&Transform, &Projection), With<Camera3d>>,
    scene: Option<Res<SceneData>>,
    mut probe_buffer: Option<ResMut<model_motion::ModelProbeLighting>>,
    mut commands: Commands,
    mut inverse_bindposes: ResMut<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
    mut joint_transforms: Query<&mut GlobalTransform, Without<Mesh3d>>,
    mut surface_transforms: Query<
        (&mut Transform, &mut GlobalTransform),
        (With<Mesh3d>, Without<Camera3d>),
    >,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut patches: ResMut<crate::rendering::bevy_backend::shader_buffer_patches::ShaderBufferPatches>,
    mut visibility: Query<&mut Visibility>,
    mut last_error: Local<Option<String>>,
    mut trace_frame: Local<u64>,
) {
    let Some(mut arms) = arms else { return };
    arms.retiring_equipment.retain(|entry| !entry.complete());
    *trace_frame += 1;
    let trace = std::env::var_os("DH2_PLAYER_VISIBILITY_TRACE").is_some();
    let renders: Cow<[ArmRender]> = if let Some(distance) = arms.debug_rest {
        match cameras.single() {
            Ok((camera, _)) => Cow::Owned(debug_rest_arms(&arms, camera, distance)),
            Err(_) => Cow::Borrowed(&[]),
        }
    } else {
        match state.as_deref().and_then(|state| state.player.as_ref()) {
            Some(player) => Cow::Borrowed(player.arms()),
            None => Cow::Borrowed(&[]),
        }
    };
    let probes = scene
        .as_ref()
        .and_then(|scene| scene.environment_probes.as_deref());
    let player = state.as_deref().and_then(|state| state.player.as_ref());
    let actions = [
        player.is_some_and(|player| {
            player
                .machine
                .is_active(crate::gameplay::player::states::StateId::MANTLE)
        }),
        player.is_some_and(|player| {
            player
                .machine
                .is_active(crate::gameplay::player::states::StateId::CATFALL)
        }),
        player.is_some_and(|player| player.is_swimming()),
    ];
    let record_swim_detail = arms.capture_geometry;
    for (index, active) in actions.into_iter().enumerate() {
        if active {
            if !arms.action_active[index] {
                arms.action_history[index] = Some(RenderActionHistory::default());
                arms.action_capture[index] = Default::default();
            }
            if let Some(history) = &mut arms.action_history[index] {
                history.frames = history.frames.saturating_add(1);
                if index == 2 {
                    if let Some(player) = player {
                        if player.is_underwater() {
                            history.swim_underwater_frames =
                                history.swim_underwater_frames.saturating_add(1);
                        } else {
                            history.swim_surface_frames =
                                history.swim_surface_frames.saturating_add(1);
                        }
                        if record_swim_detail {
                            for (side, arm) in player.body.arms.iter().take(2).enumerate() {
                                let snapshot = arm.loco.swimming_snapshot();
                                if history.swim_first[side].is_none() {
                                    history.swim_first[side] = Some(snapshot);
                                }
                                if player.is_underwater()
                                    && history.swim_first_underwater[side].is_none()
                                {
                                    history.swim_first_underwater[side] = Some(snapshot);
                                }
                                if player.body.speed > 0.001
                                    && history.swim_first_moving[side].is_none()
                                {
                                    history.swim_first_moving[side] = Some(snapshot);
                                }
                                if let Some(previous) = history.swim_last[side] {
                                    history.swim_phase_advanced[side] |=
                                        previous.progress != snapshot.progress;
                                    history.swim_idle_advanced[side] |= previous.idle
                                        == snapshot.idle
                                        && snapshot.idle.is_some()
                                        && previous.idle_elapsed != snapshot.idle_elapsed;
                                }
                                history.swim_last[side] = Some(snapshot);
                            }
                        }
                    }
                }
            }
        }
        arms.action_active[index] = active;
    }
    let capture_geometry = arms.capture_geometry;
    let clip_from_world = if capture_geometry && actions.iter().any(|active| *active) {
        cameras.single().ok().map(|(camera, projection)| {
            projection.get_clip_from_view() * camera.to_matrix().inverse()
        })
    } else {
        None
    };
    let mut error = arms
        .attachments
        .update(
            &renders,
            &mut visibility,
            &mut surface_transforms,
            probes,
            probe_buffer.as_deref_mut(),
            &mut patches,
            &mut buffers,
        )
        .err();
    let PlayerArms {
        arms: models,
        action_history,
        action_capture,
        ..
    } = &mut *arms;
    for (model_index, arm) in models.iter_mut().enumerate() {
        let render = renders.iter().find(|render| {
            render.model == arm.model
                && render.managed_entity == arm.managed_entity
                && render.visible
        });
        let skinned = render.map(|render| -> Result<(), String> {
            let palette = publish_model_skin(
                arm,
                render,
                &mut commands,
                &mut inverse_bindposes,
                &mut joint_transforms,
            )?;
            for (index, active) in actions.into_iter().enumerate() {
                if active && capture_geometry {
                    action_capture[index].push(model_index, clip_from_world, &palette);
                }
            }
            // Model vtable+88: the probe position is the centre of the
            // current world bounds. Joint 0 (`origin`, rest identity) is the
            // arm entity's frame.
            let frame = render.joints[0];
            let bounds = render_model::transformed_bounds(
                arm.info.authored_bounds,
                [frame[0][3], frame[1][3], frame[2][3]],
                std::array::from_fn(|axis| std::array::from_fn(|row| frame[row][axis])),
                [1.0; 3],
            );
            arm.world_bounds = Some(bounds);
            publish_model_lighting(
                bounds,
                &mut arm.lighting,
                &arm.surfaces,
                probes,
                probe_buffer.as_deref_mut(),
                &mut patches,
                &mut buffers,
            );
            Ok(())
        });
        if !matches!(skinned, Some(Ok(()))) {
            arm.world_bounds = None;
        }
        let show = match skinned {
            Some(Ok(())) => true,
            Some(Err(message)) => {
                error = Some(message);
                false
            }
            None => false,
        };
        let skin_error = render.is_some() && !show;
        if trace {
            let player = state.as_deref().and_then(|state| state.player.as_ref());
            let states = player
                .map(|player| player.machine.active_states())
                .unwrap_or_default();
            let animated = states.iter().any(|id| matches!(id.0, 4 | 7 | 8));
            if show != arm.shown || (animated && *trace_frame % 12 == 0) {
                let published = renders.iter().find(|render| {
                    render.model == arm.model && render.managed_entity == arm.managed_entity
                });
                eprintln!(
                    "PLAYER VISIBILITY states={states:?} model={} published={:?} joints={} skinned={} shown={} legs={:?}/{}",
                    arm.model,
                    published.map(|render| render.visible),
                    published.map_or(0, |render| render.joints.len()),
                    show,
                    arm.shown,
                    player.map(|player| player.body.legs_visible),
                    player.map_or(0, |player| player.body.desired_legs),
                );
            }
        }
        if show != arm.shown {
            for part in &arm.surfaces {
                if let Ok(mut visibility) = visibility.get_mut(part.entity) {
                    *visibility = if show {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    };
                }
            }
            arm.shown = show;
        }
        if actions.iter().any(|active| *active) {
            let visible_surfaces = arm
                .surfaces
                .iter()
                .filter(|surface| {
                    visibility
                        .get(surface.entity)
                        .is_ok_and(|value| *value == Visibility::Visible)
                })
                .count() as u32;
            for (index, active) in actions.into_iter().enumerate() {
                if !active {
                    continue;
                }
                let Some(history) = action_history[index]
                    .as_mut()
                    .and_then(|history| history.models.get_mut(model_index))
                else {
                    continue;
                };
                history.published_frames =
                    history
                        .published_frames
                        .saturating_add(u32::from(renders.iter().any(|render| {
                            render.model == arm.model && render.managed_entity == arm.managed_entity
                        })));
                history.requested_frames = history
                    .requested_frames
                    .saturating_add(u32::from(render.is_some()));
                history.skinned_frames = history.skinned_frames.saturating_add(u32::from(show));
                history.skin_error_frames = history
                    .skin_error_frames
                    .saturating_add(u32::from(skin_error));
                history.visible_surface_frames = history
                    .visible_surface_frames
                    .saturating_add(u32::from(visible_surfaces != 0));
                history.maximum_visible_surfaces =
                    history.maximum_visible_surfaces.max(visible_surfaces);
                if index == 2 && visible_surfaces != 0 {
                    if player.is_some_and(|player| player.is_underwater()) {
                        history.swim_underwater_visible_frames =
                            history.swim_underwater_visible_frames.saturating_add(1);
                    } else {
                        history.swim_surface_visible_frames =
                            history.swim_surface_visible_frames.saturating_add(1);
                    }
                }
            }
        }
    }
    if error != *last_error {
        if let Some(error) = &error {
            eprintln!("First-person arms hidden: {error}");
        }
        *last_error = error;
    }
}

#[cfg(test)]
mod tests {
    use super::arm_model_depth_hack;

    #[test]
    fn arms_inherit_the_player_depth_hack_from_the_body() {
        assert_eq!(arm_model_depth_hack(5.0), (5.0, Some(5.0)));
        // 1.0 still marks the stencil partition but publishes no parameter.
        assert_eq!(arm_model_depth_hack(1.0), (1.0, None));
        assert_eq!(arm_model_depth_hack(0.0), (0.0, None));
    }
}
