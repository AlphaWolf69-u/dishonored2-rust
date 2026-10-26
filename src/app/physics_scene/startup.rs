//! Level declaration/ownership boundaries of trigger and water startup.
use super::*;
use crate::gameplay::{
    body_activation::ActivationOwners,
    physics_world::PublicationEvent,
    signal::Signal,
    trigger_body::initialization::physics::TriggerLevelInitialization,
    trigger_body::initialization::{LoadedTriggerShape, TriggerDependency},
    water_component::startup::physics::WaterLevelStartup,
    water_contacts::{WaterContactState, WaterContactWorld, WaterDensityWorld},
};

pub(super) fn dispatch_observer(
    _: &mut Signal<usize, usize>,
    _: usize,
    _: usize,
    _: ActivationOwners<'_>,
    _: PublicationEvent<'_>,
    _: &mut TriggerComponentStore<'_>,
) -> Result<(), String> {
    Err("physics observer has no registered level handler".into())
}

pub(super) struct TriggerLevel<'a> {
    pub entity: &'a EntityComponents,
    pub index: usize,
    pub position: [f32; 3],
    pub collision: &'a crate::gameplay::trigger_body::TriggerCollisionSettings,
    pub material: u16,
    pub ranges: [[i32; 2]; 2],
    pub declarations: &'a HashMap<u64, ComponentDeclaration>,
    pub wind_class: i32,
    pub winds: &'a ComponentPool<ComponentOwner>,
    pub flags: &'a mut u8,
}
impl<F> TriggerLevelInitialization<TriggerAssets, F> for TriggerLevel<'_> {
    fn load_shape(
        &mut self,
        entity: usize,
        geometry: &mut TriggerAssets,
    ) -> Result<Option<LoadedTriggerShape>, String> {
        if entity != self.entity.identity {
            return Err("trigger shape owner differs from entity".into());
        }
        Ok(Some(LoadedTriggerShape {
            shape: *geometry
                .shape_resources
                .get(self.index)
                .ok_or("trigger shape resource absent")?,
            basis: geometry.placements[self.index].shape.basis,
        }))
    }
    fn dependency(
        &mut self,
        entity: usize,
        kind: TriggerDependency,
        components: &TriggerComponentStore<'_>,
    ) -> Result<Option<ComponentHandle>, String> {
        if entity != self.entity.identity {
            return Err("trigger dependency owner differs from entity".into());
        }
        self.entity.find_component(
            self.ranges[usize::from(kind == TriggerDependency::Wind)],
            [0x20, 0],
            |handle| {
                let owner = if handle.class_id == self.wind_class {
                    self.winds.get(handle.slot)
                } else {
                    components.component_owner(handle)
                }
                .ok_or("trigger dependency component absent")?;
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: owner.flags,
                        start_phases: Some(
                            self.declarations
                                .get(&handle.packed())
                                .ok_or("trigger dependency declaration absent")?
                                .scheduling
                                .start_phases,
                        ),
                    },
                )
            },
        )
    }
    fn position(&self, entity: usize) -> Result<[f32; 3], String> {
        if entity != self.entity.identity {
            return Err("trigger position owner differs from entity".into());
        }
        Ok(self.position)
    }
    fn collision_settings(
        &self,
        declaration: usize,
    ) -> Result<crate::gameplay::trigger_body::TriggerCollisionSettings, String> {
        if declaration != self.index {
            return Err("trigger collision declaration differs from owner".into());
        }
        Ok(*self.collision)
    }
    fn trigger_material(
        &self,
        materials: &crate::gameplay::materials::MaterialOwner<usize, usize>,
    ) -> Result<u16, String> {
        materials
            .library
            .get(self.material)
            .ok_or("trigger material not resident")?;
        Ok(self.material)
    }
    fn mark_entity_keyframed(&mut self, entity: usize) -> Result<(), String> {
        if entity != self.entity.identity {
            return Err("trigger keyframe owner differs from entity".into());
        }
        *self.flags |= 4;
        Ok(())
    }
    fn report_missing_shape(&mut self, _: usize) -> Result<(), String> {
        Err("trigger shape failed to load".into())
    }
    fn release_failed(
        &mut self,
        _: ComponentHandle,
        _: &mut WorldStorage,
        _: &mut TriggerAssets,
        _: &mut ComponentPublicationEvents<'_, usize, usize, F>,
    ) -> Result<(), String> {
        Err("trigger failed initialization; level physics publication is pending".into())
    }
}

pub(super) struct WaterLevel<'a> {
    pub entity: &'a EntityComponents,
    pub trigger_range: [i32; 2],
    pub declarations: &'a HashMap<u64, ComponentDeclaration>,
    pub scheduling: &'a DeclarationScheduling,
    pub priority: i32,
    pub contacts: &'a mut crate::gameplay::water_contacts::WaterContactManager,
}
impl<F> WaterLevelStartup<TriggerAssets, F> for WaterLevel<'_> {
    fn priorities(&self, _: ComponentHandle) -> Result<[i32; 2], String> {
        Ok([self.scheduling.priority, self.priority])
    }
    fn trigger(
        &mut self,
        entity: usize,
        components: &TriggerComponentStore<'_>,
    ) -> Result<ComponentHandle, String> {
        if entity != self.entity.identity {
            return Err("water trigger owner differs from entity".into());
        }
        self.entity
            .find_component(self.trigger_range, [0x20, 0], |handle| {
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: components.trigger(handle)?.owner.flags,
                        start_phases: Some(
                            self.declarations
                                .get(&handle.packed())
                                .ok_or("water trigger declaration absent")?
                                .scheduling
                                .start_phases,
                        ),
                    },
                )
            })?
            .ok_or_else(|| "water has no trigger component".into())
    }
    fn material_registered(&mut self, _: &mut Signal<usize, usize>, _: usize, _: usize, _: u16) {
        panic!("material observer called before level physics publication")
    }
    fn notify_contacts(
        &mut self,
        reference: ComponentHandle,
        _: &mut WorldStorage,
        _: &mut TriggerAssets,
        events: &mut ComponentPublicationEvents<'_, usize, usize, F>,
    ) -> Result<(), String> {
        self.contacts.notify_component(
            reference,
            &mut StartupContacts {
                components: &events.components,
            },
        )
    }
}

// No simulation step precedes level startup. Visit the real empty contact
// manager; a retained body here is a lifecycle violation, not a zero-density
// replacement for the body's mass, shape volume or collision user data.
struct StartupContacts<'a, 'b> {
    components: &'a TriggerComponentStore<'b>,
}
impl WaterDensityWorld for StartupContacts<'_, '_> {
    fn prepare_body(&mut self, _: u32) -> Result<(), String> {
        Err("retained body before water startup completed".into())
    }
    fn density_override(&self, _: u32) -> Result<f32, String> {
        Err("retained body before water startup completed".into())
    }
    fn body_mass(&self, _: u32) -> Result<f32, String> {
        Err("retained body before water startup completed".into())
    }
    fn shape_volume(&self, _: u32) -> Result<Option<[f32; 4]>, String> {
        Err("retained body before water startup completed".into())
    }
    fn body_label(&self, _: u32) -> Result<String, String> {
        Err("retained body before water startup completed".into())
    }
}
impl WaterContactWorld for StartupContacts<'_, '_> {
    fn water_state(&self, handle: ComponentHandle) -> Result<WaterContactState, String> {
        if handle.class_id != self.components.water_class {
            return Err("contact component is not water".into());
        }
        Ok(self
            .components
            .waters
            .get(handle.slot)
            .ok_or("water contact component absent")?
            .into())
    }
    fn deactivate_body(&mut self, _: u32) -> Result<(), String> {
        Err("retained body before water startup completed".into())
    }
}
