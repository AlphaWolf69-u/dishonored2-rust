//! 88d060's selection from an ordered, completed render-view collision query.
//! Physics supplies hit order, class/component admission and world AABBs.
mod resolve;
mod view;
pub use view::{EnvironmentViewContacts, WaterViewPublication};
mod query;
pub use query::EnvironmentViewQuery;
pub use resolve::{resolve_contacts, ContactEntity};

#[derive(Clone, Copy, Debug)]
pub struct EnvironmentContact {
    /// Present only for a trigger-derived entity with an arkCpntWater component.
    pub water_component_flags: Option<u32>,
    /// Present independently when the entity is an environment-volume type.
    pub environment: Option<EnvironmentVolumeContact>,
}

#[derive(Clone, Copy, Debug)]
pub struct EnvironmentVolumeContact {
    pub index: usize,
    pub is_water_volume: bool,
    pub bounds: [[f32; 3]; 2],
}

#[derive(Debug)]
pub struct EnvironmentContactSelection {
    pub areas: Vec<usize>,
    pub water: Option<usize>,
    pub water_level: f32,
    pub touches_water_trigger: bool,
    sizes: Vec<f32>,
}

impl EnvironmentContactSelection {
    /// The view owner supplies its retained height; a missing water hit does
    /// not overwrite it. Consumers must still check the selected water handle.
    pub fn new(water_level: f32) -> Self {
        Self {
            areas: Vec::new(),
            water: None,
            water_level,
            touches_water_trigger: false,
            sizes: Vec::new(),
        }
    }

    /// `None` covers an invalid or not-yet-ready query. It clears selection,
    /// rather than retaining last frame's environments while waiting.
    pub fn update(&mut self, contacts: Option<&[EnvironmentContact]>) {
        self.areas.clear();
        self.sizes.clear();
        self.water = None;
        self.touches_water_trigger = false;
        for contact in contacts.into_iter().flatten() {
            if contact
                .water_component_flags
                .is_some_and(|flags| flags & 0xc < 8)
            {
                self.touches_water_trigger = true;
            }
            let Some(environment) = contact.environment else {
                continue;
            };
            if environment.is_water_volume {
                self.water = Some(environment.index);
                self.water_level = environment.bounds[1][2];
            } else {
                let d: [f32; 3] = std::array::from_fn(|axis| {
                    environment.bounds[1][axis] - environment.bounds[0][axis]
                });
                let size = d[2] * d[2] + d[1] * d[1] + d[0] * d[0];
                // Native insertion searches backwards using strict '<'. Equal
                // sizes and unordered NaNs move before the earlier entries.
                let at = self
                    .sizes
                    .iter()
                    .rposition(|&prior| size < prior)
                    .map_or(0, |index| index + 1);
                self.areas.insert(at, environment.index);
                self.sizes.insert(at, size);
            }
        }
    }

    /// 88e220 publishes the inverse of this independent trigger-contact flag
    /// to ocean owner+d08, even if there is no selected water environment.
    pub fn update_ocean_height(&self) -> bool {
        !self.touches_water_trigger
    }
}

#[cfg(test)]
#[path = "environment_contacts/tests.rs"]
mod tests;
