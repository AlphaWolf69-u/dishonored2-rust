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
//! culls it). Its mesh holds world-space vertices, CPU-skinned every frame
//! from `Player::arms()` with palette = joint × inverse bind.
use super::{
    archive, bmd6, edge_skeleton, entities, game_to_view, material, model_motion,
    player::PlayerState, render_model, shadow_preparation, source_mesh, view_to_game, MaterialHandle,
    MaterialResolver, MeshTag, SceneData, ShaderBuffer,
};
use crate::gameplay::player::ArmRender;
use bevy::prelude::*;
use std::borrow::Cow;

struct ArmSurface {
    entity: Entity,
    mesh: Handle<Mesh>,
    /// Index into the model's surfaces.
    surface: usize,
    influences: u8,
    probe_index: u32,
    material: MaterialHandle,
}

struct ArmModel {
    /// The md6 decl path, as `ArmRender::model` names it.
    model: String,
    skeleton: edge_skeleton::EdgeSkeleton,
    info: bmd6::ModelInfo,
    surfaces: Vec<ArmSurface>,
    lighting: crate::probe_lighting::ProbeQuery,
    shown: bool,
    /// Current world bounds (model vtable+88), while shown.
    world_bounds: Option<[[f32; 3]; 2]>,
}

#[derive(Resource)]
pub(super) struct PlayerArms {
    /// r_playerDepthHack after the body component's start.
    pub(super) player_depth_hack: f32,
    arms: Vec<ArmModel>,
    /// `DH2_DEBUG_REST_ARMS=1`: draw the arm skeletons' rest pose 0.5 m in
    /// front of the camera instead of `Player::arms()`; another number sets
    /// that distance in metres (a diagnostic only).
    debug_rest: Option<f32>,
}

impl PlayerArms {
    /// Materials of the arms on screen, whose textures stream like those of
    /// visible scene instances.
    pub(super) fn shown_materials(&self) -> impl Iterator<Item = &MaterialHandle> {
        self.arms
            .iter()
            .filter(|arm| arm.shown)
            .flat_map(|arm| arm.surfaces.iter().map(|part| &part.material))
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
        bindings: &std::collections::HashMap<MaterialHandle, Vec<usize>>,
        demands: &[crate::texture::ProjectedAreaDemand],
    ) -> Result<(), String> {
        for arm in self.arms.iter().filter(|arm| arm.shown) {
            let Some(bounds) = arm.world_bounds else { continue };
            let (_, area) =
                crate::visibility::tome::render_query_aabb_visibility_without_depth(state, bounds)?;
            let area = area.unwrap_or(1.0);
            for part in &arm.surfaces {
                for &slot in bindings.get(&part.material).into_iter().flatten() {
                    demands
                        .get(slot)
                        .ok_or("arm material references an unknown image")?
                        .submit(area);
                }
            }
        }
        Ok(())
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
    meshes: &mut Assets<Mesh>,
    commands: &mut Commands,
) -> Result<PlayerArms, String> {
    let text = |path: &str| -> Result<Option<String>, String> {
        Ok(catalog
            .read(path)
            .map_err(|e| e.to_string())?
            .map(|bytes| entities::declaration_text(&bytes).into_owned()))
    };
    let character = character_name(super::pause_menu::debug_character());
    let declarations = entities::player::player_declarations(character, text)?;
    let view = entities::player::player_view_models(&declarations.body, text)?;
    // The arm entities and the body entity's own model (drawn only while
    // the legs are visible).
    let arm_hack = arm_model_depth_hack(view.depth_hack);
    let models = view
        .arms
        .into_iter()
        .flatten()
        .map(|model| (model, arm_hack))
        .chain(std::iter::once((declarations.body_model.clone(), body_model_depth_hack(view.depth_hack))));
    let mut arms = Vec::new();
    for (model, (model_depth_hack, parameter)) in models {
        let stem = model
            .strip_suffix(".md6")
            .ok_or_else(|| format!("arm model {model} is not an md6"))?;
        let resource = format!("generated/basemodel/{stem}.bmd6model");
        let declaration = |path: &str| text(path).ok().flatten();
        let mesh_path = bmd6::resolve_compiled_mesh(&resource, declaration)?;
        let bytes = catalog
            .read(&mesh_path)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("{mesh_path} is absent"))?;
        let info = bmd6::inspect(&bytes)?;
        let binding = bmd6::resolve_skeleton_binding(&resource, declaration)?;
        let skeleton_name = binding
            .as_ref()
            .map(|binding| binding.name.as_str())
            .filter(|name| !name.is_empty())
            .or_else(|| (!info.skeleton_name.is_empty()).then_some(info.skeleton_name.as_str()))
            .ok_or_else(|| format!("{model} has no skeleton binding"))?;
        let skeleton = shadow_preparation::read_skeleton(catalog, skeleton_name)?;
        if skeleton.inverse_bind.len() < skeleton.rest.len() {
            return Err(format!("{skeleton_name} has fewer inverse binds than joints"));
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
        let palette =
            edge_skeleton::skin_palette(&rest, &skeleton.inverse_bind[..rest.len()])?;
        let mut surfaces = Vec::new();
        for (index, surface) in info.surfaces.iter().enumerate() {
            if material::is_null_material(&surface.material_path)
                || surface.positions.is_empty()
                || surface.indices.is_empty()
            {
                continue;
            }
            let material_path = if model == declarations.body_model {
                view.visible_legs_materials
                    .get(index)
                    .map(String::as_str)
                    .unwrap_or(&surface.material_path)
            } else {
                &surface.material_path
            };
            let Some(resolved) = resolver.get(material_path, 0, [0.0; 3]) else {
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
            let mut mesh =
                source_mesh(&surface.positions, &surface.uvs, &surface.indices, &surface.packed_channels, false);
            shadow_preparation::skin_surface(
                &mut mesh,
                surface,
                &palette,
                influences,
                |p| game_to_view(p.to_array()),
                |v| game_to_view(v.to_array()),
            )
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
            // Re-skinned on the CPU every frame: keep the main-world copy.
            mesh.asset_usage = bevy::asset::RenderAssetUsages::default();
            let mesh = meshes.add(mesh);
            let entity = commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(handle.clone()),
                    Transform::IDENTITY,
                    GlobalTransform::IDENTITY,
                    Visibility::Hidden,
                    MeshTag(resolved.probe_index),
                    // addAlways.
                    bevy::camera::visibility::NoFrustumCulling,
                    bevy::light::NotShadowCaster,
                ))
                .id();
            surfaces.push(ArmSurface {
                entity,
                mesh,
                surface: index,
                influences,
                probe_index: resolved.probe_index,
                material: resolved.handle.clone(),
            });
        }
        arms.push(ArmModel {
            model,
            skeleton,
            info,
            surfaces,
            lighting: Default::default(),
            shown: false,
            world_bounds: None,
        });
    }
    Ok(PlayerArms {
        player_depth_hack: view.depth_hack,
        arms,
        debug_rest: std::env::var("DH2_DEBUG_REST_ARMS").ok().and_then(|value| {
            if value == "1" { Some(0.5) } else { value.parse().ok() }
        }),
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
    let origin = Vec3::from_array(view_to_game(camera.translation)) + forward * distance - up * 0.15;
    let rotation = Quat::from_mat3(&Mat3::from_cols(left, up, forward));
    let root = edge_skeleton::JointRestPose {
        rotation: rotation.to_array(),
        translation: origin.to_array(),
        scale: [1.0; 3],
    };
    arms.arms
        .iter()
        .filter_map(|arm| {
            let joints =
                edge_skeleton::pose_matrices(&arm.skeleton.rest, &arm.skeleton.hierarchy, &root)
                    .ok()?;
            Some(ArmRender { model: arm.model.clone(), joints, visible: true })
        })
        .collect()
}

fn write_probe_record(buffer: &mut ShaderBuffer, tag: u32, record: [[f32; 4]; 9]) {
    if tag & 0x8000_0000 != 0 {
        return;
    }
    let Some(data) = buffer.data.as_mut() else { return };
    let start = tag as usize * 9 * 16;
    let Some(bytes) = data.get_mut(start..start + 9 * 16) else { return };
    for (coefficient, channels) in record.iter().enumerate() {
        for (channel, value) in channels.iter().enumerate() {
            let at = coefficient * 16 + channel * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
}

/// Skin and show the arms `Player::arms()` names (left, then right); hide
/// arm models it omits or marks invisible.
pub(super) fn update_player_arms(
    state: Option<Res<PlayerState>>,
    arms: Option<ResMut<PlayerArms>>,
    cameras: Query<&Transform, With<Camera3d>>,
    scene: Option<Res<SceneData>>,
    probe_buffer: Option<Res<model_motion::ModelProbeLighting>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut visibility: Query<&mut Visibility>,
    mut last_error: Local<Option<String>>,
) {
    let Some(mut arms) = arms else { return };
    let renders: Cow<[ArmRender]> = if let Some(distance) = arms.debug_rest {
        match cameras.single() {
            Ok(camera) => Cow::Owned(debug_rest_arms(&arms, camera, distance)),
            Err(_) => Cow::Borrowed(&[]),
        }
    } else {
        match state.as_deref().and_then(|state| state.player.as_ref()) {
            Some(player) => Cow::Borrowed(player.arms()),
            None => Cow::Borrowed(&[]),
        }
    };
    let probes = scene.as_ref().and_then(|scene| scene.environment_probes.as_deref());
    let mut error = None;
    for arm in &mut arms.arms {
        let render = renders
            .iter()
            .find(|render| render.model == arm.model && render.visible);
        let skinned = render.map(|render| -> Result<(), String> {
            let count = arm.skeleton.rest.len();
            if render.joints.len() != count {
                return Err(format!(
                    "{} pose has {} joints, skeleton {count}",
                    arm.model,
                    render.joints.len()
                ));
            }
            let palette =
                edge_skeleton::skin_palette(&render.joints, &arm.skeleton.inverse_bind[..count])?;
            for part in &arm.surfaces {
                let surface = &arm.info.surfaces[part.surface];
                let mut mesh = meshes
                    .get_mut(&part.mesh)
                    .ok_or_else(|| format!("{} mesh is missing", arm.model))?;
                shadow_preparation::skin_surface(
                    &mut mesh,
                    surface,
                    &palette,
                    part.influences,
                    |p| game_to_view(p.to_array()),
                    |v| game_to_view(v.to_array()),
                )?;
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
            let center = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5);
            if let (Some(probes), Some(handle)) = (probes, probe_buffer.as_ref()) {
                if let Some(lighting) = probes.interpolate_query_sh(center, &mut arm.lighting) {
                    if let Some(mut buffer) = buffers.get_mut(&handle.0) {
                        let record = lighting.object_shader_record();
                        for part in &arm.surfaces {
                            write_probe_record(&mut buffer, part.probe_index, record);
                        }
                    }
                }
            }
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
        if show != arm.shown {
            for part in &arm.surfaces {
                if let Ok(mut visibility) = visibility.get_mut(part.entity) {
                    *visibility = if show { Visibility::Visible } else { Visibility::Hidden };
                }
            }
            arm.shown = show;
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
