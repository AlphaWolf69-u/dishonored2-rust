//! Water component4d6850 startup; declaration and world resources stay owned
//! by the level. Every runtime boundary is required, including contact updates.
use super::WaterComponent;
use crate::gameplay::component_state::entity::ComponentHandle;
pub(crate) mod physics;

pub struct WaterDeclaration {
    pub materials: [Option<usize>; 2],
    pub is_ocean: bool,
    pub density: f32,
    pub current: [f32; 3],
}

pub trait WaterStartupWorld {
    fn water(&self, reference: ComponentHandle) -> Result<&WaterComponent, String>;
    fn water_mut(&mut self, reference: ComponentHandle) -> Result<&mut WaterComponent, String>;
    fn admit(&mut self, reference: ComponentHandle) -> Result<(), String>;
    ///877720 resolves the component once; its movement body can change during
    /// bounds/material callbacks and must be read again before registration.
    fn trigger(&mut self, entity: usize) -> Result<ComponentHandle, String>;
    fn trigger_body(&self, trigger: ComponentHandle) -> Result<u32, String>;
    /// Main shape calcAabb on the body's live transform, not its packed bounds.
    fn shape_bounds(&mut self, body: u32) -> Result<[[f32; 4]; 2], String>;
    fn material_flags(&self, material: usize) -> Result<u8, String>;
    fn resolve_material(&mut self, material: usize) -> Result<u16, String>;
    fn set_material(&mut self, body: u32, material: u16) -> Result<(), String>;
    ///651d30/650bf0 visits existing water-contact bodies after publication.
    fn notify_contacts(&mut self, reference: ComponentHandle) -> Result<(), String>;
}

/// Publish through the canonical pool so material/contact callbacks can resolve
/// this same component without taking it out of its owner or copying its state.
pub fn start_water(
    reference: ComponentHandle,
    declaration: &WaterDeclaration,
    world: &mut impl WaterStartupWorld,
) -> Result<(), String> {
    world.admit(reference)?;
    let trigger = world.trigger(
        world
            .water(reference)?
            .owner
            .entity
            .ok_or("water component has no entity")?,
    )?;
    let body = world.trigger_body(trigger)?;
    let bounds = world.shape_bounds(body)?;
    world.water_mut(reference)?.plane =
        Some([0., 0., 1., crate::float::subtract(0., bounds[1][2])]);
    let selected = usize::from(declaration.is_ocean);
    if let Some(material) = declaration.materials[selected] {
        if world.material_flags(material)? & 0x20 == 0 {
            return Err("water requires a trigger physics material".into());
        }
        let body = world.trigger_body(trigger)?;
        let material = world.resolve_material(material)?;
        world.set_material(body, material)?;
    }
    world.water_mut(reference)?.flow_and_density = Some([
        declaration.current[0],
        declaration.current[1],
        declaration.current[2],
        declaration.density,
    ]);
    world.notify_contacts(reference)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct World {
        water: WaterComponent,
        priority: i32,
        entity_priority: i32,
        bounds: [[f32; 4]; 2],
        material: u16,
        events: Vec<u32>,
    }
    impl WaterStartupWorld for World {
        fn water(&self, reference: ComponentHandle) -> Result<&WaterComponent, String> {
            assert_eq!(reference.slot, 0);
            Ok(&self.water)
        }
        fn water_mut(&mut self, reference: ComponentHandle) -> Result<&mut WaterComponent, String> {
            assert_eq!(reference.slot, 0);
            Ok(&mut self.water)
        }
        fn admit(&mut self, reference: ComponentHandle) -> Result<(), String> {
            let priority = self.priority;
            let entity_priority = self.entity_priority;
            let owner = &mut self.water_mut(reference)?.owner;
            if crate::gameplay::component_state::prepare_admission(
                &mut owner.flags,
                u8::from(priority < 8),
                priority,
                entity_priority,
            ) {
                self.events.push(0);
            }
            Ok(())
        }
        fn trigger(&mut self, entity: usize) -> Result<ComponentHandle, String> {
            assert_eq!(entity, 0);
            self.events.push(1);
            Ok(ComponentHandle {
                class_id: 1,
                slot: 0,
            })
        }
        fn trigger_body(&self, trigger: ComponentHandle) -> Result<u32, String> {
            assert_eq!(
                trigger,
                ComponentHandle {
                    class_id: 1,
                    slot: 0
                }
            );
            Ok(0x01000001)
        }
        fn shape_bounds(&mut self, body: u32) -> Result<[[f32; 4]; 2], String> {
            assert_eq!(body, 0x01000001);
            self.events.push(2);
            Ok(self.bounds)
        }
        fn material_flags(&self, material: usize) -> Result<u8, String> {
            assert!(material < 2);
            assert!(self.water.plane().is_some());
            assert!(self.water.flow_and_density().is_none());
            Ok(0x20)
        }
        fn resolve_material(&mut self, material: usize) -> Result<u16, String> {
            self.events.extend([3, material as u32]);
            Ok(11 + material as u16)
        }
        fn set_material(&mut self, body: u32, material: u16) -> Result<(), String> {
            assert_eq!(body, 0x01000001);
            assert!(self.water.plane().is_some());
            assert!(self.water.flow_and_density().is_none());
            self.material = material;
            self.events.push(4);
            Ok(())
        }
        fn notify_contacts(&mut self, reference: ComponentHandle) -> Result<(), String> {
            let water = self.water(reference)?;
            assert!(water.plane().is_some() && water.flow_and_density().is_some());
            self.events.push(5);
            Ok(())
        }
    }
    #[test]
    fn starts_match_executed_admission_material_and_parameter_publication() {
        for row in include_str!("../../../tests/fixtures/water_component_start.txt").lines() {
            let mut words = row
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap());
            let flags = words.next().unwrap();
            let priority = words.next().unwrap() as i32;
            let entity_priority = words.next().unwrap() as i32;
            let selected = words.next().unwrap() != 0;
            let materials = std::array::from_fn(|i| (words.next().unwrap() != 0).then_some(i));
            let parameters: [f32; 4] =
                std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
            let bounds = std::array::from_fn(|_| {
                std::array::from_fn(|_| f32::from_bits(words.next().unwrap()))
            });
            let mut water = WaterComponent::default();
            water.owner.flags = flags;
            water.owner.entity = Some(0);
            assert!(water.flow_and_density().is_none());
            let mut world = World {
                water,
                priority,
                entity_priority,
                bounds,
                material: 0,
                events: Vec::new(),
            };
            let reference = ComponentHandle {
                class_id: 0,
                slot: 0,
            };
            start_water(
                reference,
                &WaterDeclaration {
                    materials,
                    is_ocean: selected,
                    density: parameters[0],
                    current: [parameters[1], parameters[2], parameters[3]],
                },
                &mut world,
            )
            .unwrap();
            let water = &world.water;
            let mut actual = vec![water.owner.flags];
            actual.extend(water.plane().unwrap().map(f32::to_bits));
            actual.extend(water.flow_and_density().unwrap().map(f32::to_bits));
            actual.extend([u32::from(world.material), world.events.len() as u32]);
            actual.extend(world.events);
            assert_eq!(actual, words.collect::<Vec<_>>());
        }
    }
}
