//! 4f5360/4f5bf0/4f6d50: immutable open-owned inventory projection.
//! Definitions retain their canonical source leases; this is not an inventory
//! cache. The worker and gameplay owner share the same short-lived row array.
use crate::assets::inventory_item::PickableItem;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Item {
    pub equipment: Arc<PickableItem>,
    pub ammunition: Option<Arc<PickableItem>>,
    pub upgrade: Option<Arc<crate::assets::equipment::attachment::Upgrade>>,
    pub count: i32,
    pub maximum: i32,
    pub visible: bool,
}
impl Item {
    pub fn display(&self) -> &PickableItem {
        self.ammunition.as_deref().unwrap_or(&self.equipment)
    }
    pub fn index(&self) -> i32 {
        self.display().hud_index
    }
    pub fn label(&self) -> &str {
        self.upgrade
            .as_ref()
            .map_or(&self.display().name, |upgrade| &upgrade.name)
    }
    pub fn description(&self) -> &str {
        self.upgrade
            .as_ref()
            .map_or(&self.display().description, |upgrade| &upgrade.description)
    }
}

#[derive(Clone)]
pub(crate) struct Snapshot {
    pub items: Arc<[Item]>,
    pub shortcuts: [i32; 14],
    pub current: i32,
    pub can_health: bool,
    pub can_mana: bool,
    pub health_count: u32,
    pub mana_count: u32,
}
#[derive(Clone, Copy)]
pub(crate) struct Potions {
    pub can_health: bool,
    pub can_mana: bool,
    pub health_count: u32,
    pub mana_count: u32,
}
impl Snapshot {
    pub fn potions(&self) -> Potions {
        Potions {
            can_health: self.can_health,
            can_mana: self.can_mana,
            health_count: self.health_count,
            mana_count: self.mana_count,
        }
    }
}

///4f4bc0 constructs these exact scales/durations. The installed state leaves
///m_bendTimeInfo at these defaults; requests compose in ClockRuntime.
#[derive(Clone, Copy)]
pub(crate) struct Bend {
    pub world: f32,
    pub unbent: f32,
    pub enter: f32,
    pub exit: f32,
}
impl Default for Bend {
    fn default() -> Self {
        Self {
            world: f32::from_bits(0x3d4c_cccd),
            unbent: f32::from_bits(0x3d4c_cccd),
            enter: 0.,
            exit: 0.,
        }
    }
}
