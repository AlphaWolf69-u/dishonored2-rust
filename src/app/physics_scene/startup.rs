//! Level declaration/ownership boundaries of trigger and water startup.
use super::*;
#[path = "activation_events.rs"]
mod activation_events;
use crate::gameplay::{
    signal::Signal,
    trigger_body::initialization::physics::TriggerLevelInitialization,
    trigger_body::initialization::{LoadedTriggerShape, TriggerDependency},
    water_component::startup::physics::WaterLevelStartup,
};
pub(super) use activation_events::{
    WorldActivationObserver, install as install_activation_observers,
};

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
            shape: geometry
                .shape_resources
                .get(self.index)
                .copied()
                .flatten()
                .ok_or("trigger shape resource absent")?,
            basis: geometry.placements[self.index]
                .as_ref()
                .ok_or("trigger placement retired")?
                .shape
                .basis,
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
    pub notify_contacts: &'a mut bool,
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
        _reference: ComponentHandle,
        _: &mut WorldStorage,
        _: &mut TriggerAssets,
        _events: &mut ComponentPublicationEvents<'_, usize, usize, F>,
    ) -> Result<(), String> {
        *self.notify_contacts = true;
        Ok(())
    }
}
