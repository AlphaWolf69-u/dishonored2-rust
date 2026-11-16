//!651020 SurfaceImpact material views. Declarations remain in the canonical
//! physics material owner; no resource acquisition occurs during force work.
use super::*;
impl PhysicsScene {
    pub(in crate::app) fn water_impact_material_revision(&self) -> Result<usize, String> {
        Ok(self
            .world
            .read()
            .map_err(|_| "water impact material world poisoned")?
            .storage
            .materials
            .declaration_revision())
    }
    pub(in crate::app) fn water_impact_material_pairs(
        &self,
    ) -> Result<Vec<(String, String)>, String> {
        let world = self
            .world
            .read()
            .map_err(|_| "water impact material world poisoned")?;
        let owner = &world.storage.materials;
        let mut targets = std::collections::HashSet::new();
        for declaration in &self.water_declarations {
            let name = declaration.materials[usize::from(declaration.is_ocean)]
                .map(|handle| {
                    owner
                        .get(handle)
                        .map(|material| material.name.clone())
                        .ok_or("water impact declaration absent")
                })
                .transpose()?
                .unwrap_or_else(|| "default_preset".to_owned());
            targets.insert(name);
        }
        let mut pairs = Vec::new();
        for source in owner.contact_definitions() {
            for target in &targets {
                pairs.push((source.material().name.clone(), target.clone()));
            }
        }
        Ok(pairs)
    }
    pub(in crate::app) fn water_impact_material_names(
        &self,
        impact: &water_simulation::WaterImpact,
    ) -> Result<(String, String), String> {
        let world = self
            .world
            .read()
            .map_err(|_| "water impact material world poisoned")?;
        let owner = &world.storage.materials;
        Ok((
            owner.material_for_id(impact.source_material)?.name.clone(),
            impact
                .target_material
                .map(|id| {
                    owner
                        .material_for_id(id)
                        .map(|material| material.name.clone())
                })
                .transpose()?
                .unwrap_or_else(|| "default_preset".to_owned()),
        ))
    }
}
