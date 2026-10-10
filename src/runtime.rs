//! Mutable level state, separate from rendering and input devices.
//! This is a small reimplementation, not a transcription of game code.
use crate::entities::Pickup;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionError {
    UnknownEntity,
    AlreadyCollected,
    InventoryOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldEvent {
    PickupCollected {
        entity_id: String,
        item_path: String,
        quantity: u32,
        inventory_total: u64,
    },
}

#[derive(Debug)]
struct PickupState {
    item_path: String,
    quantity: u32,
    available: bool,
}

#[derive(Debug, Default)]
pub struct WorldState {
    pickups: HashMap<String, PickupState>,
    inventory: HashMap<String, u64>,
}

impl WorldState {
    pub fn from_pickups(definitions: &[Pickup]) -> Result<Self, String> {
        let mut world = Self::default();
        for definition in definitions {
            if definition.entity_id.is_empty()
                || definition.item_path.is_empty()
                || definition.quantity == 0
            {
                return Err(format!(
                    "invalid pickup definition: {}",
                    definition.entity_id
                ));
            }
            let replaced = world.pickups.insert(
                definition.entity_id.clone(),
                PickupState {
                    item_path: definition.item_path.clone(),
                    quantity: definition.quantity,
                    available: true,
                },
            );
            if replaced.is_some() {
                return Err(format!("duplicate pickup entity: {}", definition.entity_id));
            }
        }
        Ok(world)
    }

    pub fn pickup_available(&self, entity_id: &str) -> bool {
        self.pickups
            .get(entity_id)
            .is_some_and(|pickup| pickup.available)
    }

    pub fn inventory_quantity(&self, item_path: &str) -> u64 {
        self.inventory.get(item_path).copied().unwrap_or(0)
    }

    pub fn collect_pickup(&mut self, entity_id: &str) -> Result<WorldEvent, InteractionError> {
        let pickup = self
            .pickups
            .get_mut(entity_id)
            .ok_or(InteractionError::UnknownEntity)?;
        if !pickup.available {
            return Err(InteractionError::AlreadyCollected);
        }
        let total = self
            .inventory
            .get(&pickup.item_path)
            .copied()
            .unwrap_or(0)
            .checked_add(pickup.quantity as u64)
            .ok_or(InteractionError::InventoryOverflow)?;
        pickup.available = false;
        self.inventory.insert(pickup.item_path.clone(), total);
        Ok(WorldEvent::PickupCollected {
            entity_id: entity_id.to_string(),
            item_path: pickup.item_path.clone(),
            quantity: pickup.quantity,
            inventory_total: total,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pickup(id: &str, item: &str, quantity: u32) -> Pickup {
        Pickup {
            entity_id: id.into(),
            position: [0.0; 3],
            item_path: item.into(),
            quantity,
        }
    }

    #[test]
    fn collection_changes_entity_and_inventory_once() {
        let mut world = WorldState::from_pickups(&[
            pickup("coin_a", "coins/low", 1),
            pickup("coin_b", "coins/low", 2),
        ])
        .unwrap();
        assert_eq!(world.inventory_quantity("coins/low"), 0);
        assert_eq!(
            world.collect_pickup("coin_a"),
            Ok(WorldEvent::PickupCollected {
                entity_id: "coin_a".into(),
                item_path: "coins/low".into(),
                quantity: 1,
                inventory_total: 1,
            })
        );
        assert!(!world.pickup_available("coin_a"));
        assert_eq!(
            world.collect_pickup("coin_a"),
            Err(InteractionError::AlreadyCollected)
        );
        assert_eq!(
            world.collect_pickup("coin_b"),
            Ok(WorldEvent::PickupCollected {
                entity_id: "coin_b".into(),
                item_path: "coins/low".into(),
                quantity: 2,
                inventory_total: 3,
            })
        );
        assert_eq!(world.inventory_quantity("coins/low"), 3);
        assert_eq!(
            world.collect_pickup("absent"),
            Err(InteractionError::UnknownEntity)
        );
    }

    #[test]
    fn rejects_duplicate_or_invalid_definitions() {
        assert!(
            WorldState::from_pickups(&[pickup("a", "item", 1), pickup("a", "item", 1)]).is_err()
        );
        assert!(WorldState::from_pickups(&[pickup("a", "item", 0)]).is_err());
    }
}
