//! Prepared ordinary ocean environment inputs. Player water-contact selection
//! is a separate ordered physics-query result, not this camera-area stack.
use super::OceanSettings;
use crate::{
    assets::{archive::Catalog, entities::EnvironmentRenderParameters},
    rendering::{
        environment::AreaEnvironment,
        environment_parameters::{EnvironmentParameters, EnvironmentTracks, EnvironmentValue},
    },
};

pub struct OceanEnvironment {
    world: EnvironmentParameters,
    areas: Vec<(String, EnvironmentParameters)>,
    tracks: EnvironmentTracks,
    image_sources: Vec<String>,
}
impl OceanEnvironment {
    pub fn load(
        catalog: &Catalog,
        world: &EnvironmentRenderParameters,
        areas: &[AreaEnvironment],
    ) -> Result<Self, String> {
        let mut world =
            EnvironmentParameters::capture(catalog, world, |read| OceanSettings::resolve(read))?;
        let mut defaults = EnvironmentParameters::defaults(catalog, &world)?;
        let mut prepared = areas
            .iter()
            .map(|area| {
                Ok((
                    area.id.clone(),
                    EnvironmentParameters::supplied(catalog, &area.parameters, &world)?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        // Native images compare resident identity. Resolve aliases once, before
        // tracking; authored and registered aliases of one image must not blend.
        let mut image_sources = std::collections::BTreeSet::new();
        for values in std::iter::once(&mut defaults)
            .chain(std::iter::once(&mut world))
            .chain(prepared.iter_mut().map(|(_, p)| p))
        {
            for value in values.values.values_mut() {
                if let EnvironmentValue::Image(name) = value {
                    let resource = crate::material::image_load_resource(catalog, name)
                        .ok_or_else(|| format!("unresolved ocean image {name}"))?;
                    *name = resource.to_ascii_lowercase();
                    image_sources.insert(name.clone());
                }
            }
        }
        Ok(Self {
            world,
            areas: prepared,
            tracks: EnvironmentTracks::new(&defaults),
            image_sources: image_sources.into_iter().collect(),
        })
    }
    pub fn image_sources(&self) -> &[String] {
        &self.image_sources
    }
    pub fn update(
        &mut self,
        areas: &[AreaEnvironment],
        eye: [f32; 3],
        world_duration: f32,
        delta: f32,
    ) -> Result<OceanSettings, String> {
        if areas.len() != self.areas.len()
            || areas
                .iter()
                .zip(&self.areas)
                .any(|(a, (id, _))| &a.id != id)
        {
            return Err("ocean areas changed without preparing their inputs".into());
        }
        let active = areas
            .iter()
            .enumerate()
            .filter(|(_, area)| area.contains(eye))
            .map(|(i, area)| (i, &self.areas[i].1, area.blend_duration))
            .collect::<Vec<_>>();
        OceanSettings::resolve(
            self.tracks
                .update(&self.world, world_duration, &active, delta)?,
        )
    }
}

#[cfg(test)]
#[path = "environment/tests.rs"]
mod tests;
