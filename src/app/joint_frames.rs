//! Demand-only attachment frames from the same poses used for skinning.
//! 879930/256080 resolve tags/joints; 53d3f0 and 53de90 compose bindings.
use std::collections::HashMap;
use bevy::prelude::Resource;
use crate::{archive::Catalog, entities::PlacedModel, gameplay::reference_frame::{self, ReferenceFrame}};

#[derive(Clone, Copy)]
pub(super) struct JointBinding {
    pub joint: i16,
    pub offset: [f32; 3],
    pub orientation: [f32; 4],
    pub from_tag: bool,
}

struct Frames {
    initial: ReferenceFrame,
    current: ReferenceFrame,
    /// Host demand ownership: attachment1, actual Audio joint slot2.
    owners: u8,
}

struct PlacementFrames {
    scale: [f32; 3],
    /// (bound child's model property group, tag); None is _info.
    bindings: HashMap<(Option<String>, String), JointBinding>,
    joints: HashMap<i16, Frames>,
}

/// Host projection of already acquired/sampled skeleton resources. Only actual
/// attachment demands retain matrices; rigs, complete palettes and source text
/// are not duplicated. The ordinary level resource owner drops this map.
#[derive(Resource, Default)]
pub(super) struct JointFrames {
    placements: HashMap<usize, PlacementFrames>,
}

impl JointFrames {
    ///53e610->09d1e0 copies skeleton+30 rest records into the animator;
    ///09dc40/09e240 compose its hierarchy, then53e440 initializes phase0/3
    ///banks from component+198. Actual started layers override that bank in
    ///the same lower->scene->physics order used during gameplay.
    fn initial_matrices(placement: usize, rig: &crate::assets::animation_rig::Rig,
        machinery: &super::machinery::AmbientMachinery,
        animations: &super::usable_animation::UsableAnimations)
        -> Result<Vec<super::physics_skinning::Affine>, String> {
        match machinery.initial_joint_matrices(placement, &rig.skeleton, animations)? {
            Some(matrices) => Ok(matrices),
            None => crate::edge_skeleton::pose_matrices(&rig.skeleton.rest, &rig.skeleton.hierarchy, &rig.root),
        }
    }
    pub(super) fn prepare(catalog: &Catalog, models: &[PlacedModel], requests: &[(usize, String, Option<String>)],
        machinery: &super::machinery::AmbientMachinery,
        animations: &super::usable_animation::UsableAnimations) -> Result<Self, String> {
        let mut result = Self::default();
        let mut rigs = HashMap::new();
        let mut initial = HashMap::new();
        for (placement, name, group) in requests {
            let model = models.get(*placement).ok_or("joint attachment placement is absent")?;
            let rig = if let Some(rig) = animations.prepared_rig(*placement) { rig } else { match rigs.entry(model.resource_path.clone()) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let declaration = crate::bmd6::declaration_path(&model.resource_path)
                        .ok_or("joint attachment parent is not an MD6 model")?;
                    let path = declaration.strip_prefix("generated/decls/md6def/")
                        .and_then(|path| path.strip_suffix(".decl")).ok_or("joint attachment MD6 path is invalid")?;
                    entry.insert(crate::assets::animation_rig::Rig::acquire(catalog, path)?)
                },
            } };
            let binding = match rig.optional_tag_quaternion_in_group(group.as_deref().unwrap_or("_info"), name)? {
                Some((joint, offset, orientation)) => JointBinding {
                    joint: i16::try_from(joint).map_err(|_| "bound joint index exceeds signed representation")?,
                    offset: offset.to_array(), orientation: orientation.to_array(), from_tag: true,
                },
                //752900/878470 retain joint=-1 when neither actual MD6 tag
                //nor bone exists; the hierarchy then binds parent-relative.
                None => JointBinding { joint: rig.skeleton.joint_names.iter().position(|joint| joint == name)
                    .map(|joint| i16::try_from(joint).map_err(|_| "bound joint index exceeds signed representation"))
                    .transpose()?.unwrap_or(-1),
                    offset: [0.; 3], orientation: [0., 0., 0., 1.], from_tag: false },
            };
            let frames = result.placements.entry(*placement).or_insert_with(|| PlacementFrames {
                scale: model.scale, bindings: HashMap::new(), joints: HashMap::new(),
            });
            if binding.joint < 0 {
                frames.bindings.insert((group.clone(), name.clone()), binding);
                continue;
            }
            if let std::collections::hash_map::Entry::Vacant(entry) = frames.joints.entry(binding.joint) {
                let matrices = match initial.entry(*placement) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::hash_map::Entry::Vacant(entry) => entry.insert(Self::initial_matrices(*placement, &rig, machinery, animations)?),
                };
                let matrix = *usize::try_from(binding.joint).ok().and_then(|joint| matrices.get(joint))
                    .ok_or("bound joint is outside initialized pose bank")?;
                let initial = matrix_frame(matrix);
                entry.insert(Frames { initial, current: initial, owners: 1 });
            }
            frames.bindings.insert((group.clone(), name.clone()), binding);
        }
        Ok(result)
    }

    /// Marker tags are optional (676c90/6777d0). Acquire through the existing
    /// rig owner behind loading; retain only demanded joint frames. Temporary
    /// rest palettes and rig leases end before publishing the level.
    pub(super) fn prepare_optional_tags(&mut self, catalog: &Catalog, models: &[PlacedModel],
        requests: &[(usize, String)], machinery: &super::machinery::AmbientMachinery,
        animations: &super::usable_animation::UsableAnimations) -> Result<(), String> {
        let mut rigs = HashMap::new();
        let mut initial = HashMap::new();
        for (placement, name) in requests {
            if self.resolve(*placement, None, name)?.is_some() { continue; }
            let model = models.get(*placement).ok_or("marker placement is absent")?;
            let Some(declaration) = crate::bmd6::declaration_path(&model.resource_path) else { continue; };
            let rig = if let Some(rig) = animations.prepared_rig(*placement) { rig } else {
                match rigs.entry(model.resource_path.clone()) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let (binding, _) = crate::assets::animation_rig::declaration::resolve_binding(catalog,
                            &model.resource_path, crate::assets::animation_rig::declaration::LoadContext::Level)?;
                        if !binding.is_some_and(|binding| binding.name.is_some()) { continue; }
                        let path = declaration.strip_prefix("generated/decls/md6def/")
                            .and_then(|path| path.strip_suffix(".decl")).ok_or("marker MD6 path is invalid")?;
                        entry.insert(crate::assets::animation_rig::Rig::acquire(catalog, path)?)
                    }
                }
            };
            let Some((joint, offset, orientation)) = rig.optional_tag_quaternion_in_group("_info", name)? else { continue; };
            let joint = i16::try_from(joint).map_err(|_| "marker joint exceeds signed representation")?;
            let matrices = match initial.entry(*placement) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(Self::initial_matrices(*placement, &rig, machinery, animations)?),
            };
            let matrix = *usize::try_from(joint).ok().and_then(|joint| matrices.get(joint))
                .ok_or("marker tag joint is outside initialized pose bank")?;
            let initial = matrix_frame(matrix);
            let frames = self.placements.entry(*placement).or_insert_with(|| PlacementFrames {
                scale: model.scale, bindings: HashMap::new(), joints: HashMap::new(),
            });
            frames.joints.entry(joint).and_modify(|frame| frame.owners |= 1)
                .or_insert(Frames { initial, current: initial, owners: 1 });
            frames.bindings.insert((None, name.clone()), JointBinding { joint, offset: offset.to_array(), orientation: orientation.to_array(), from_tag: true });
        }
        Ok(())
    }

    ///709f70 borrows the animator bank. Admit only actual sound-joint demands;
    ///the existing final-pose publisher updates this same retained frame.
    pub(super) fn admit_audio(&mut self, placement: usize, scale: [f32; 3], joint: i16,
        initial: super::physics_skinning::Affine) -> Result<(), String> {
        if joint < 0 { return Err("default audio object is not a skeleton demand".into()); }
        let entry = self.placements.entry(placement).or_insert_with(|| PlacementFrames {
            scale, bindings: HashMap::new(), joints: HashMap::new(),
        });
        if entry.scale != scale { return Err("audio and attachment placement scales differ".into()); }
        let initial = matrix_frame(initial);
        entry.joints.entry(joint).and_modify(|frame| frame.owners |= 2)
            .or_insert(Frames { initial, current: initial, owners: 2 });
        Ok(())
    }

    pub(super) fn release_audio(&mut self, placement: usize, joint: i16) {
        let Some(entry) = self.placements.get_mut(&placement) else { return; };
        if let Some(frame) = entry.joints.get_mut(&joint) { frame.owners &= !2; }
        entry.joints.retain(|_, frame| frame.owners != 0);
        if entry.joints.is_empty() && entry.bindings.is_empty() { self.placements.remove(&placement); }
    }

    /// `group` is the bound child's model (879890's property selector).
    pub(super) fn resolve(&self, placement: usize, group: Option<&str>, tag: &str) -> Result<Option<JointBinding>, String> {
        Ok(self.placements.get(&placement).and_then(|entry| entry.bindings.iter()
            .find(|((g, t), _)| g.as_deref() == group && t == tag)).map(|(_, binding)| *binding))
    }

    pub(super) fn frame(&self, placement: usize, joint: i16, phase: u8) -> Result<Option<ReferenceFrame>, String> {
        let Some(frame) = self.placements.get(&placement).and_then(|entry| entry.joints.get(&joint)) else { return Ok(None); };
        match phase {
            0 => Ok(Some(frame.initial)),
            3 => Ok(Some(frame.current)),
            _ => Err(format!("joint attachment phase {phase} has no sampled pose owner")),
        }
    }

    pub(super) fn local_frame(&self, placement: usize, joint: i16, phase: u8,
        offset: [f32; 3], orientation: [f32; 4])
        -> Result<Option<ReferenceFrame>, String> {
        let Some(frame) = self.frame(placement, joint, phase)? else { return Ok(None); };
        let scale = self.placements.get(&placement).ok_or("joint attachment placement disappeared")?.scale;
        Ok(Some(reference_frame::joints::bind_local_frame(frame, scale, offset, orientation)))
    }

    pub(super) fn bound_frame(&self, placement: usize, joint: i16, phase: u8,
        offset: [f32; 3], orientation: [f32; 4], parent_world: ReferenceFrame)
        -> Result<Option<ReferenceFrame>, String> {
        let Some(local) = self.local_frame(placement, joint, phase, offset, orientation)? else { return Ok(None); };
        let mut world = reference_frame::compose(parent_world, local);
        world.basis = reference_frame::joints::normalize_basis(world.basis);
        Ok(Some(world))
    }

    /// Called at the existing final-pose boundary, before inverse-bind. There
    /// is no new evaluation, allocation, rig lookup or whole-palette copy.
    pub(super) fn publish(&mut self, placement: usize, current: &[[[f32; 4]; 3]]) {
        if self.placements.is_empty() { return; }
        let Some(entry) = self.placements.get_mut(&placement) else { return; };
        for (&joint, frames) in &mut entry.joints {
            if let Some(matrix) = usize::try_from(joint).ok().and_then(|index| current.get(index)) {
                frames.current = matrix_frame(*matrix);
            }
        }
    }
}

fn matrix_frame(matrix: [[f32; 4]; 3]) -> ReferenceFrame {
    ReferenceFrame { position: [matrix[0][3], matrix[1][3], matrix[2][3]],
        basis: std::array::from_fn(|column| [matrix[0][column], matrix[1][column], matrix[2][column]]) }
}

/// 784e30 uses879890's optional tag and53dac0's phase3 frame. Borrow the
/// actual AnimBase palette; no second skeleton, pose evaluation or cache.
pub(super) fn current_tag_frame(rig: &crate::assets::animation_rig::Rig,
    matrices: &[[[f32; 4]; 3]], group: &str, name: &str, scale: [f32; 3],
    parent: ReferenceFrame) -> Result<Option<ReferenceFrame>, String> {
    let Some((joint, offset, orientation)) = rig.optional_tag_quaternion_in_group(group, name)? else {
        return Ok(None);
    };
    let Some(matrix) = matrices.get(joint) else { return Ok(None); };
    let local = reference_frame::joints::bind_local_frame(matrix_frame(*matrix), scale,
        offset.to_array(), orientation.to_array());
    Ok(Some(reference_frame::compose(parent, local)))
}
