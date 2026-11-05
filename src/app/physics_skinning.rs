//! Skin simulated physics-system MD6 placements from their bodies each frame.
//! A ragdoll-mapped joint follows its body: joint = body * map, the inverse
//! of 14076a180's body-from-joint keyframing (140ae3540) and exact for the
//! installed cinfo poses against the skeleton rest pose. The other joints
//! keep their rest offsets from their parents.
use super::{
    shadow_updates::ModelShadowSources, RenderSourcePlacements, SceneRenderQueue, ShaderBuffer,
};
use crate::{archive::Catalog, assets::edge_skeleton, bmd6, gameplay::machinery_world::MachineryWorld};
use bevy::prelude::*;
use std::sync::Arc;

pub(super) type Affine = [[f32; 4]; 3];

pub(super) struct SystemSkin {
    system: usize,
    placement: usize,
    parents: edge_skeleton::SharedSlice<Option<usize>>,
    rest: edge_skeleton::SharedSlice<Affine>,
    inverse_bind: edge_skeleton::SharedSlice<Affine>,
    model: Arc<bmd6::ModelInfo>,
    publication: super::skin_publication::SkinPublication,
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

/// Animation may contain authored scale. Unlike body transforms, these
/// matrices are not necessarily rigid and require a full affine inverse.
fn inverse_animation(m: &Affine) -> Result<Affine, String> {
    let basis = Mat3::from_cols(Vec3::new(m[0][0], m[1][0], m[2][0]),
        Vec3::new(m[0][1], m[1][1], m[2][1]), Vec3::new(m[0][2], m[1][2], m[2][2]));
    let determinant = basis.determinant();
    if !determinant.is_finite() || determinant == 0.0 {
        return Err("animated parent joint transform is singular".into());
    }
    let inverse = basis.inverse();
    let translation = -(inverse * Vec3::new(m[0][3], m[1][3], m[2][3]));
    let columns = [inverse.x_axis, inverse.y_axis, inverse.z_axis];
    Ok(std::array::from_fn(|row| [columns[0][row], columns[1][row], columns[2][row], translation[row]]))
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
    let mut rigs = edge_skeleton::RestRigSources::new(catalog);
    let mut models = bmd6::ModelSources::new(catalog);
    for (index, system) in world.systems.iter().enumerate() {
        let mut load = || -> Result<SystemSkin, String> {
            let rig = rigs.acquire(catalog, &system.skeleton)?;
            let placement = placements.get(system.placement).ok_or("missing system placement")?;
            let model = models.acquire(catalog, &placement.resource_path)?;
            Ok(SystemSkin {
                system: index,
                placement: system.placement,
                parents: rig.parents,
                inverse_bind: rig.inverse_bind,
                rest: rig.rest,
                model,
                publication: Default::default(),
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

    pub(super) fn model(&self) -> &Arc<bmd6::ModelInfo> {
        &self.model
    }

    fn current(&self, world: &MachineryWorld) -> Vec<Affine> {
        self.current_with_animation(world, None).expect("rigid rest transform")
    }

    /// 14076d640 installs world-space modifications only for physics-driven
    /// joints; 14053bcd0 composes their untouched descendants from the active
    /// animation pose rather than resetting them to their bind pose.
    fn current_with_animation(&self, world: &MachineryWorld, animation: Option<&[Affine]>) -> Result<Vec<Affine>, String> {
        let base = animation.filter(|pose| pose.len() == self.rest.len()).unwrap_or(&self.rest);
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
        // Preserve each unmodified joint's current local animation relative
        // to its parent, applying any physics override higher in the tree.
        let mut out = base.to_vec();
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
                        let parent_inverse = if animation.is_some() { inverse_animation(&base[p])? } else { inverse(&base[p]) };
                        let local = multiply(&parent_inverse, &base[j]);
                        out[j] = multiply(&out[p], &local);
                        done[j] = true;
                    }
                } else {
                    done[j] = true;
                }
            }
        }
        Ok(out)
    }

    pub(super) fn animated_palette(&self, world: &MachineryWorld, animation: &[Affine]) -> Result<Vec<Affine>, String> {
        if animation.len() != self.rest.len() { return Err("animation and physics skeleton joint counts differ".into()); }
        let current = self.current_with_animation(world, Some(animation))?;
        edge_skeleton::skin_palette(&current, &self.inverse_bind)
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
    animated: Option<&super::usable_animation::UsableAnimations>,
) {
    for skin in skins {
        if animated.is_some_and(|animations| animations.combines_physics(skin.placement)) { continue; }
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
        super::skin_publication::publish_target(
            placement, source, &skin.model, palette, &mut skin.publication, meshes, buffers, queue,
        );
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
