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
}

struct PlacementFrames {
    scale: [f32; 3],
    bindings: HashMap<String, JointBinding>,
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
    pub(super) fn prepare(catalog: &Catalog, models: &[PlacedModel], requests: &[(usize, String)],
        machinery: &super::machinery::AmbientMachinery) -> Result<Self, String> {
        let mut result = Self::default();
        let mut rigs = HashMap::new();
        for (placement, name) in requests {
            let model = models.get(*placement).ok_or("joint attachment placement is absent")?;
            let rig = match rigs.entry(model.resource_path.clone()) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let declaration = crate::bmd6::declaration_path(&model.resource_path)
                        .ok_or("joint attachment parent is not an MD6 model")?;
                    let path = declaration.strip_prefix("generated/decls/md6def/")
                        .and_then(|path| path.strip_suffix(".decl")).ok_or("joint attachment MD6 path is invalid")?;
                    entry.insert(crate::assets::animation_rig::Rig::acquire(catalog, path)?)
                },
            };
            let binding = match rig.optional_tag_quaternion_in_group("_info", name)? {
                Some((joint, offset, orientation)) => JointBinding {
                    joint: i16::try_from(joint).map_err(|_| "bound joint index exceeds signed representation")?,
                    offset: offset.to_array(), orientation: orientation.to_array(), from_tag: true,
                },
                None => JointBinding { joint: i16::try_from(rig.joint(name)?)
                    .map_err(|_| "bound joint index exceeds signed representation")?,
                    offset: [0.; 3], orientation: [0., 0., 0., 1.], from_tag: false },
            };
            let frames = result.placements.entry(*placement).or_insert_with(|| PlacementFrames {
                scale: model.scale, bindings: HashMap::new(), joints: HashMap::new(),
            });
            if let std::collections::hash_map::Entry::Vacant(entry) = frames.joints.entry(binding.joint) {
                // These admitted attachments are driven by the existing physics
                // skin owner. Sample its real startup body/joint composition,
                // never substitute a rest joint when a producer is absent.
                let matrix = machinery.initial_joint_matrix(*placement, binding.joint, &rig.skeleton)?
                    .ok_or_else(|| format!("{} joint {name} has no prepared pose producer", model.id))?;
                let initial = matrix_frame(matrix);
                entry.insert(Frames { initial, current: initial });
            }
            frames.bindings.insert(name.clone(), binding);
        }
        Ok(result)
    }

    pub(super) fn resolve(&self, placement: usize, tag: &str) -> Result<Option<JointBinding>, String> {
        Ok(self.placements.get(&placement).and_then(|entry| entry.bindings.get(tag)).copied())
    }

    pub(super) fn frame(&self, placement: usize, joint: i16, phase: u8) -> Result<Option<ReferenceFrame>, String> {
        let Some(frame) = self.placements.get(&placement).and_then(|entry| entry.joints.get(&joint)) else { return Ok(None); };
        match phase {
            0 => Ok(Some(frame.initial)),
            3 => Ok(Some(frame.current)),
            _ => Err(format!("joint attachment phase {phase} has no sampled pose owner")),
        }
    }

    pub(super) fn bound_frame(&self, placement: usize, joint: i16, phase: u8,
        offset: [f32; 3], orientation: [f32; 4], parent_world: ReferenceFrame)
        -> Result<Option<ReferenceFrame>, String> {
        let Some(frame) = self.frame(placement, joint, phase)? else { return Ok(None); };
        let scale = self.placements.get(&placement).ok_or("joint attachment placement disappeared")?.scale;
        let local = reference_frame::joints::bind_local_frame(frame, scale, offset, orientation);
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
