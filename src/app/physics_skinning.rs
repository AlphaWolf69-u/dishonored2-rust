//! Skin simulated physics-system MD6 placements from their bodies each frame.
//! A ragdoll-mapped joint follows its body: joint = body * map, the inverse
//! of 14076a180's body-from-joint keyframing (140ae3540) and exact for the
//! installed cinfo poses against the skeleton rest pose. The other joints
//! keep their rest offsets from their parents.
use super::{
    apply_rest_pose, render_bounds, shadow_updates::ModelShadowSources, shadow_updates::RestSkinning,
    RenderModelSurface, RenderSourcePlacements, SceneRenderQueue, ShaderBuffer,
};
use crate::{archive::Catalog, assets::edge_skeleton, bmd6, gameplay::machinery_world::MachineryWorld};
use bevy::prelude::*;
use std::sync::Arc;

type Affine = [[f32; 4]; 3];

pub(super) struct SystemSkin {
    system: usize,
    placement: usize,
    parents: Vec<Option<usize>>,
    rest: Vec<Affine>,
    inverse_bind: Vec<Affine>,
    model: bmd6::ModelInfo,
    /// The placement's skinned instances and palette, bound on first use.
    bound: Option<(std::ops::Range<usize>, Handle<ShaderBuffer>)>,
}

fn multiply(a: &Affine, b: &Affine) -> Affine {
    std::array::from_fn(|r| {
        std::array::from_fn(|c| {
            let t = if c == 3 { a[r][3] } else { 0. };
            ((a[r][1] * b[1][c] + a[r][0] * b[0][c]) + a[r][2] * b[2][c]) + t
        })
    })
}

fn inverse(m: &Affine) -> Affine {
    let mut out = [[0.; 4]; 3];
    for r in 0..3 {
        for c in 0..3 {
            out[r][c] = m[c][r];
        }
    }
    for r in 0..3 {
        out[r][3] = -((out[r][1] * m[1][3] + out[r][0] * m[0][3]) + out[r][2] * m[2][3]);
    }
    out
}

/// An hkTransform (columns and translation) as a row-major 3x4.
fn rows(columns: &[[f32; 4]; 4]) -> Affine {
    edge_skeleton::joint_matrix(*columns)
}

pub(super) fn load_skins(
    catalog: &Catalog,
    world: &MachineryWorld,
    placements: &[crate::entities::PlacedModel],
) -> Vec<SystemSkin> {
    let mut skins = Vec::new();
    for (index, system) in world.systems.iter().enumerate() {
        let load = || -> Result<SystemSkin, String> {
            let stem = system
                .skeleton
                .strip_suffix(".md6skl")
                .ok_or("unsupported skeleton resource suffix")?;
            let path = format!("generated/skeleton/{stem}.edgeskl");
            let bytes = catalog.read(&path).map_err(|e| e.to_string())?.ok_or_else(|| format!("{path} is absent"))?;
            let skeleton = edge_skeleton::parse(&bytes)?;
            let root = edge_skeleton::JointRestPose {
                rotation: [0., 0., 0., 1.],
                translation: [0.; 3],
                scale: [1.; 3],
            };
            let rest = edge_skeleton::pose_matrices(&skeleton.rest, &skeleton.hierarchy, &root)?;
            let placement = placements.get(system.placement).ok_or("missing system placement")?;
            let read = |decl: &str| {
                catalog.read(decl).ok().flatten().map(|b| String::from_utf8_lossy(&b).into_owned())
            };
            let path = bmd6::resolve_compiled_mesh(&placement.resource_path, read)?;
            let bytes = catalog.read(&path).map_err(|e| e.to_string())?.ok_or_else(|| format!("{path} is absent"))?;
            let model = bmd6::inspect(&bytes)?;
            Ok(SystemSkin {
                system: index,
                placement: system.placement,
                parents: skeleton.parents.clone(),
                inverse_bind: skeleton.inverse_bind[..rest.len()].to_vec(),
                rest,
                model,
                bound: None,
            })
        };
        match load() {
            Ok(skin) => skins.push(skin),
            Err(error) => warn!("Physics-system skin unavailable: {error}"),
        }
    }
    skins
}

impl SystemSkin {
    pub(super) fn placement(&self) -> usize {
        self.placement
    }

    fn current(&self, world: &MachineryWorld) -> Vec<Affine> {
        let system = &world.systems[self.system];
        let entity = {
            let basis = crate::physworld::quaternion_basis(system.orientation);
            let p = system.position;
            rows(&[basis[0], basis[1], basis[2], [p[0], p[1], p[2], 1.]])
        };
        let to_model = inverse(&entity);
        let mut current: Vec<Option<Affine>> = vec![None; self.rest.len()];
        for (entry, &body) in system.ragdoll.iter().zip(&system.bodies) {
            let joint = entry.joint as usize;
            if joint < current.len() {
                let body = rows(&world.bodies[body].body_transform());
                let map = rows(&entry.transform);
                current[joint] = Some(multiply(&to_model, &multiply(&body, &map)));
            }
        }
        // Rest-relative offsets for unmapped joints, parents first.
        let mut out = self.rest.clone();
        let mut done = vec![false; out.len()];
        for _ in 0..out.len() {
            for j in 0..out.len() {
                if done[j] {
                    continue;
                }
                if let Some(m) = current[j] {
                    out[j] = m;
                    done[j] = true;
                } else if let Some(p) = self.parents.get(j).copied().flatten() {
                    if done[p] {
                        let local = multiply(&inverse(&self.rest[p]), &self.rest[j]);
                        out[j] = multiply(&out[p], &local);
                        done[j] = true;
                    }
                } else {
                    done[j] = true;
                }
            }
        }
        out
    }
}

/// After the machinery step: re-pose each simulated placement's surfaces
/// and publish its palette to the skinned shadow path.
#[allow(clippy::too_many_arguments)]
pub(super) fn pose_physics_systems(
    skins: &mut [SystemSkin],
    world: &MachineryWorld,
    meshes: &mut Assets<Mesh>,
    buffers: &mut Assets<ShaderBuffer>,
    queue: &mut SceneRenderQueue,
    sources: &mut ModelShadowSources,
    placements: &RenderSourcePlacements,
) {
    for skin in skins {
        let Some(placement) = placements.0.get(skin.placement) else {
            continue;
        };
        let Some(source) = sources.0.iter_mut().find(|s| s.placement == skin.placement) else {
            continue;
        };
        let current = skin.current(world);
        let Ok(palette) = edge_skeleton::skin_palette(&current, &skin.inverse_bind) else {
            continue;
        };
        // Setup gives deformed placements their own meshes that keep their
        // main-world data (AmbientMachinery::deformed_placements).
        let (instances, buffer) = skin.bound.get_or_insert_with(|| {
            let gpu: Vec<[Vec4; 3]> = palette.iter().map(|m| m.map(Vec4::from_array)).collect();
            (source.instances.clone(), buffers.add(ShaderBuffer::from(gpu)))
        });
        let mut points = Vec::new();
        for index in instances.clone() {
            let instance = &mut queue.instances[index];
            let handle = instance.mesh.clone();
            let Some(RenderModelSurface::Bmd6 { surface, .. }) = instance.source.map(|s| s.surface) else {
                continue;
            };
            let Some(info) = skin.model.surfaces.get(surface) else { continue };
            if info.skin_influences == 0 {
                continue;
            }
            let baked = instance.source.is_some_and(|s| s.world_baked);
            if let Some(mut mesh) = meshes.get_mut(&handle) {
                if apply_rest_pose(&mut mesh, info, &palette, info.skin_influences, placement, baked).is_ok() {
                    if let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
                        let local: Vec<Vec3> = p.iter().map(|v| Vec3::from_array(*v)).collect();
                        let world: Vec<Vec3> = if baked {
                            local
                        } else {
                            local.iter().map(|v| instance.transform.transform_point(*v)).collect()
                        };
                        instance.bounds = render_bounds(&world);
                        points.extend(world);
                    }
                }
            }
        }
        if let Some(mut gpu) = buffers.get_mut(&*buffer) {
            *gpu = ShaderBuffer::from(palette.iter().map(|m| m.map(Vec4::from_array)).collect::<Vec<_>>());
        }
        source.skinning = Some(Ok(Arc::new(RestSkinning { palette: buffer.clone(), matrices: palette })));
        if !points.is_empty() {
            let bounds = render_bounds(&points);
            let (min, max) = (Vec3::from(bounds.min()), Vec3::from(bounds.max()));
            let a = super::view_to_game(min);
            let b = super::view_to_game(max);
            source.bounds = [std::array::from_fn(|i| a[i].min(b[i])), std::array::from_fn(|i| a[i].max(b[i]))];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires installed Dust District archives"]
    fn installed_dust_cable_joints_start_at_the_rest_pose() {
        let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        let level = crate::assets::level::LevelResources::resolve(&catalog, "dust_p_lowchaos").unwrap();
        let bytes = level.read_entities(&catalog).unwrap().unwrap();
        let document = crate::entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
        let placements =
            super::super::expanded_model_placements(&crate::entities::render_placements(&document), &catalog);
        let recipes = crate::entities::machinery::recipes::load(&catalog, &document, &placements).unwrap();
        let world = MachineryWorld::from_recipes(&recipes).unwrap();
        let skins = load_skins(&catalog, &world, &placements);
        assert_eq!(skins.len(), 3);
        for skin in &skins {
            let current = skin.current(&world);
            let worst = current
                .iter()
                .zip(&skin.rest)
                .flat_map(|(a, b)| a.iter().flatten().zip(b.iter().flatten()).map(|(x, y)| (x - y).abs()))
                .fold(0f32, f32::max);
            println!("{}: {} joints, worst spawn deviation from rest {worst}", placements[skin.placement].id, current.len());
            assert!(worst < 2e-3, "joints derived from bodies leave the rest pose by {worst}");
        }
    }

    #[test]
    #[ignore = "requires installed Dust District archives"]
    fn installed_dust_speakers_follow_their_simulated_turnbuckles() {
        let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        let level = crate::assets::level::LevelResources::resolve(&catalog, "dust_p_lowchaos").unwrap();
        let bytes = level.read_entities(&catalog).unwrap().unwrap();
        let document = crate::entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
        let placements =
            super::super::expanded_model_placements(&crate::entities::render_placements(&document), &catalog);
        let recipes = crate::entities::machinery::recipes::load(&catalog, &document, &placements).unwrap();
        let world = MachineryWorld::from_recipes(&recipes).unwrap();
        let binds = crate::entities::entity_binds(&document);
        let speakers: Vec<_> = binds.iter().filter(|b| b.parent.starts_with("ld_kar_speaker_turnbuckle")).collect();
        assert_eq!(speakers.len(), 3);
        for bind in speakers {
            let parent = placements.iter().position(|p| p.id == bind.parent).expect("turnbuckle placement");
            assert!(
                world.bodies.iter().any(|b| b.placement == Some(parent)),
                "{} is not a simulated body",
                bind.parent
            );
            let child = placements.iter().position(|p| p.id == bind.entity_id).expect("speaker placement");
            let mut bound = super::super::model_motion::BoundModels::new(binds.clone());
            let mut poses = vec![super::super::model_motion::ModelPose { placement: parent, delta: [[1., 0., 0., 0.5], [0., 1., 0., 0.], [0., 0., 1., 0.]] }];
            bound.propagate(&mut poses, &placements);
            assert!(poses.iter().any(|p| p.placement == child), "{} does not follow {}", bind.entity_id, bind.parent);
        }
    }
}
