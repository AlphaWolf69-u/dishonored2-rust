//! HUDGrenade45af50 uses the ordinary entity/tag binding, not another pose.
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
    ) -> Result<(), String> {
        if tag.is_empty() {
            return self.bind_entity(
                live,
                child,
                parent,
                super::throwable::BindingPose {
                    joint: -1,
                    offset: [0.; 3],
                    orientation: [0., 0., 0., 1.],
                    parent_placement: None,
                },
                events,
            );
        }
        let tree = player
            .body
            .equipment
            .iter()
            .flatten()
            .find(|tree| tree.managed_entity == Some(parent))
            .ok_or("GrenadeCooking parent has no actual equipment tree")?;
        let (joint, offset, orientation) = tree.rig.tag(tag)?;
        let joint =
            i16::try_from(joint).map_err(|_| "GrenadeCooking tag index exceeds joint ABI")?;
        let (position, axis) = player
            .equipment_joint_world(
                parent,
                usize::try_from(joint).map_err(|_| "invalid equipment tag joint")?,
            )
            .ok_or("GrenadeCooking parent joint has no published pose")?;
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
                offset: offset.to_array(),
                orientation: bevy_math::Quat::from_mat3(&orientation).to_array(),
                parent_placement: None,
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
