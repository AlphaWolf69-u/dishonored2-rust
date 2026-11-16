//! c87700/c878d0/c87970: quality/user-flag publication on the existing body.
//! Every accepted request retains its exact identity and stage across observer
//! failure. Low motion-type flags are never changed by this policy owner.
use super::*;
use crate::gameplay::physics_world::TransformCallbacks;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Policy {
    Quality(u8),
    Flags { mask: u32, enabled: bool },
}
pub(super) struct PolicyProgress {
    body: u32,
    policy: Policy,
    mode: i32,
    stage: u8,
}
impl WorldStorage {
    ///089fb0 drains the real accepted API FIFO before the physical solve.
    ///Only completed dispatch resets retained command chunks; errors preserve
    ///the source command-buffer cursor and all remaining body requests.
    pub(crate) fn dispatch_body_commands<
        G: BodyCreationGeometry,
        E: PublicationEvents + BodyCreationCallbacks,
    >(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        commands: &mut crate::gameplay::physics_world::commands::PhysicsCommands,
    ) -> Result<(), String> {
        if !self.publication_idle() {
            return Err("body publication must finish before command dispatch".into());
        }
        self.edit_bodies(geometry, events, |creation| {
            commands.dispatch(creation.world, creation.cells, creation.callbacks)
        })?;
        commands.reset(false)
    }
    pub(crate) fn set_body_quality<
        G: BodyCreationGeometry,
        E: PublicationEvents + BodyCreationCallbacks,
    >(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        id: u32,
        quality: u8,
        mode: i32,
    ) -> Result<bool, String> {
        self.change_body_policy(geometry, events, id, Policy::Quality(quality), mode)
    }
    pub(crate) fn set_body_user_flags<
        G: BodyCreationGeometry,
        E: PublicationEvents + BodyCreationCallbacks,
    >(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        id: u32,
        mask: u32,
        enabled: bool,
        mode: i32,
    ) -> Result<bool, String> {
        self.change_body_policy(
            geometry,
            events,
            id,
            Policy::Flags {
                mask: mask & !15,
                enabled,
            },
            mode,
        )
    }
    fn change_body_policy<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        id: u32,
        policy: Policy,
        mode: i32,
    ) -> Result<bool, String> {
        let mut progress = if let Some(progress) = self.policy_progress.take() {
            if progress.body != id || progress.policy != policy || progress.mode != mode {
                self.policy_progress = Some(progress);
                return Err("another body policy publication is incomplete".into());
            }
            progress
        } else {
            if !self.publication_idle() {
                return Err("body publication must finish before a policy change".into());
            }
            let body = self.bodies.get(id).ok_or("body policy identity is stale")?;
            let changed = match policy {
                Policy::Quality(quality) => {
                    self.qualities
                        .get(quality)
                        .ok_or("body quality preset is absent")?;
                    body.quality_id != quality
                }
                Policy::Flags { mask, enabled } => {
                    if enabled {
                        body.flags & mask != mask
                    } else {
                        body.flags & mask != 0
                    }
                }
            };
            if !changed {
                return Ok(false);
            }
            self.bodies.validate_collision_invalidation(id)?;
            PolicyProgress {
                body: id,
                policy,
                mode,
                stage: 0,
            }
        };
        let result = self.edit_bodies(geometry, events, |creation| {
            if progress.stage == 0 {
                let body = creation
                    .world
                    .bodies
                    .get_mut(id)
                    .ok_or("body policy identity retired")?;
                match policy {
                    Policy::Quality(quality) => body.quality_id = quality,
                    Policy::Flags { mask, enabled } => {
                        body.flags = if enabled {
                            body.flags | mask
                        } else {
                            body.flags & !mask
                        };
                        // Nativeworld+478 broadphase body-change callback.
                        creation.world.broadphase.mark_bodies([id]);
                    }
                }
                progress.stage = 1;
            }
            if progress.stage == 1 {
                if mode == 0 {
                    creation
                        .world
                        .invalidate_collision_cache(id, 0, creation.callbacks)?;
                }
                progress.stage = 2;
            }
            if progress.stage == 2 {
                creation.callbacks.transform_changed(
                    crate::gameplay::body_activation::ActivationOwners {
                        bodies: &mut *creation.world.bodies,
                        motions: &mut *creation.world.motions,
                        deactivation: &mut *creation.world.deactivation,
                        broadphase: &mut *creation.world.broadphase,
                    },
                    id,
                    false,
                )?;
                progress.stage = 3;
            }
            Ok(true)
        });
        if result.is_err() {
            self.policy_progress = Some(progress);
        }
        result
    }
}
