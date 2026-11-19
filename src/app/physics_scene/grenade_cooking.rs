//! HUDGrenade45bc10/45af50 binds to the held equipment's actual arm parent.
use super::*;
use crate::gameplay::{
    entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities},
    player::Player,
    reference_frame::{ReferenceFrame, relative_binding},
};

#[derive(Debug)]
pub(in crate::app) struct GrenadeObservation {
    pub entity: EntityId,
    pub total: f32,
    pub progress: f32,
    pub armed: bool,
    pub exploded: bool,
    pub body_shape: Option<(u32, u8)>,
}

impl PhysicsScene {
    ///Opt-in diagnostic only: borrow the canonical component/body/shape
    ///owners, never admit an actor or retain another resource lease.
    pub(in crate::app) fn debug_grenades(
        &self,
        live: &LiveEntities,
    ) -> Result<Vec<GrenadeObservation>, String> {
        let mut result = Vec::new();
        let physics = self
            .world
            .read()
            .map_err(|_| "grenade diagnostic physics owner poisoned")?;
        for &slot in &self.grenades.roster.indices[..self.grenades.roster.used as usize] {
            let grenade = self
                .grenades
                .get(slot)
                .ok_or("grenade diagnostic component absent")?;
            let Some(index) = grenade.owner.entity else {
                continue;
            };
            let entity = self.entities[index].shared;
            if live.get(entity).is_none() {
                continue;
            }
            let body_shape = self
                .document_rigid(index)?
                .map(|handle| self.rigid_component(handle))
                .transpose()?
                .and_then(|component| component.body())
                .and_then(|id| physics.storage.bodies.get(id))
                .map(|body| {
                    let shape = body.selected_shape(0)?;
                    let asset = physics
                        .geometry
                        .shapes
                        .get(shape)
                        .and_then(Option::as_ref)
                        .ok_or("grenade diagnostic live body shape absent")?;
                    Ok::<_, String>((body.id, asset.header.shape_type))
                })
                .transpose()?;
            result.push(GrenadeObservation {
                entity,
                total: grenade.total,
                progress: grenade.progress(),
                armed: grenade.armed(),
                exploded: grenade.exploded(),
                body_shape,
            });
        }
        Ok(result)
    }
    pub(in crate::app) fn grenade_cooking_owner(&self) -> usize {
        self.owner_id
    }
    pub(in crate::app) fn grenade_cooking_definition(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<Option<Arc<crate::assets::grenades::Definition>>, String> {
        Ok(self.grenade_component(live, id)?.map(|component| {
            self.grenades
                .get(component.handle.slot)
                .unwrap()
                .definition
                .clone()
        }))
    }
    pub(in crate::app) fn grenade_cooking_state(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<Option<(f32, f32, bool, bool)>, String> {
        let Some(component) = self.grenade_component(live, id)? else {
            return Ok(None);
        };
        let value = self
            .grenades
            .get(component.handle.slot)
            .ok_or("Grenade cooking source absent")?;
        Ok(Some((
            value.total,
            value.progress(),
            value.armed(),
            value.exploded(),
        )))
    }
    pub(in crate::app) fn bind_cooking_equipment(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        child: EntityId,
        parent: EntityId,
        player: &Player,
        tag: &str,
        child_model: Option<&str>,
    ) -> Result<(), String> {
        let child_frame = self
            .entity_frame(live, child)?
            .ok_or("GrenadeCooking child frame pending")?;
        let parent_frame = self
            .entity_frame(live, parent)?
            .ok_or("GrenadeCooking parent frame pending")?;
        let arm = player
            .body
            .arms
            .iter()
            .find(|arm| arm.managed_entity == Some(parent));
        let tree = player
            .body
            .equipment
            .iter()
            .flatten()
            .find(|tree| tree.managed_entity == Some(parent));
        let rig = if let Some(arm) = arm {
            Some(
                arm.rig
                    .as_deref()
                    .ok_or("GrenadeCooking arm rig readiness absent")?,
            )
        } else {
            tree.map(|tree| tree.rig.as_ref())
        };
        //875e80→752900→879930 selects the bound CHILD model's property
        //group (25bfe0 falls back to _info only when that group is absent).
        //A missing/unbound tag next tries the actual skeleton joint name.
        let tag_binding = match rig.filter(|_| !tag.is_empty()) {
            Some(rig) => {
                rig.optional_tag_quaternion_in_group(child_model.unwrap_or("_info"), tag)?
            }
            None => None,
        };
        let selected_joint = tag_binding
            .as_ref()
            .map(|(joint, _, _)| *joint)
            .or_else(|| {
                rig.filter(|_| !tag.is_empty())
                    .and_then(|rig| rig.skeleton.joint_names.iter().position(|name| name == tag))
            });
        let Some(joint) = selected_joint else {
            //752900's diagnostic166830 returns; joint remains -1 and753182
            //preserves the actual world pose by computing parent-local rows.
            //An empty tag/non-skeletal parent enters this same source branch.
            let (offset, orientation) = relative_binding(parent_frame, child_frame);
            return self.bind_entity(
                live,
                child,
                parent,
                super::throwable::BindingPose {
                    joint: -1,
                    offset,
                    orientation,
                    parent_placement: None,
                    initial_pose: None, //Missing tag/bare parent retains current world pose.
                },
                events,
            );
        };
        let joint =
            i16::try_from(joint).map_err(|_| "GrenadeCooking tag index exceeds joint ABI")?;
        let (position, axis) = player
            .equipment_joint_world(
                parent,
                usize::try_from(joint).map_err(|_| "invalid equipment tag joint")?,
            )
            .ok_or("GrenadeCooking parent joint has no published pose")?;
        let joint_frame = ReferenceFrame {
            position: position.to_array(),
            basis: axis.to_cols_array_2d(),
        };
        //A real tag snaps to its authored transform; a bare joint preserves
        //the pre-bind world transform when no explicit offsets were supplied.
        let (offset, orientation) = if let Some((_, offset, rotation)) = tag_binding {
            (offset.to_array(), rotation.to_array())
        } else {
            relative_binding(joint_frame, child_frame)
        };
        let index = *self
            .shared_indices
            .get(&parent)
            .ok_or("GrenadeCooking parent Scene generation absent")?;
        let (local_position, local_rotation) = relative_binding(
            self.entities[index].frame.pose,
            ReferenceFrame {
                position: position.to_array(),
                basis: axis.to_cols_array_2d(),
            },
        );
        let previous = self.document_hierarchy.external_joints.insert(
            (index, joint),
            crate::gameplay::reference_frame::bind(
                ReferenceFrame::default(),
                local_position,
                local_rotation,
            ),
        );
        let result = self.bind_entity(
            live,
            child,
            parent,
            super::throwable::BindingPose {
                joint,
                offset,
                orientation,
                parent_placement: None,
                initial_pose: Some(crate::gameplay::reference_frame::bind(joint_frame,offset,orientation)),
            },
            events,
        );
        if result.is_err()
            && !self
                .document_hierarchy
                .bindings
                .iter()
                .any(|binding| binding.parent == index && binding.joint == joint)
        {
            if let Some(previous) = previous {
                self.document_hierarchy
                    .external_joints
                    .insert((index, joint), previous);
            } else {
                self.document_hierarchy
                    .external_joints
                    .remove(&(index, joint));
            }
        }
        result
    }
}
