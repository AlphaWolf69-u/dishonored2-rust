//! Authored area profiles, including the water volumes selected by 88d060.
//! Query hits and physical AABBs remain with the physics/contact owner.
use crate::{archive, entities};

pub struct EnvironmentVolume {
    pub placement: entities::PlacedEnvironmentVolume,
    pub parameters: entities::EnvironmentRenderParameters,
    pub is_water_volume: bool,
    ///88e220 reads this nullable Material parameter from the selected water
    /// profile after ordinary environment tracking, without blending it.
    pub surface_material: Option<String>,
}

pub(super) fn resolve(
    placement: entities::PlacedEnvironmentVolume,
    catalog: &archive::Catalog,
) -> Result<EnvironmentVolume, String> {
    let parameters = entities::environment_render_parameters_with_inheritance(
        &placement.id,
        &placement.environment_decl,
        |parent| {
            catalog
                .read(&format!("generated/decls/env/{parent}.decl"))
                .map(|v| v.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
                .map_err(|e| e.to_string())
        },
    )?;
    // 342ce0's default argument is false in 88d060. Use the same parameter
    // decoder as the existing ordinary-area path, independently of containment.
    let is_water_volume = parameters
        .scalar_parameter("water/IsWaterVolume")?
        .is_some_and(|v| v != 0.0);
    let surface_material = if is_water_volume {
        crate::assets::environment::ParameterReader {
            catalog,
            parameters: &parameters,
        }
        .material_source("water/SurfaceMaterial")?
    } else {
        None
    };
    Ok(EnvironmentVolume {
        surface_material,
        placement,
        parameters,
        is_water_volume,
    })
}

/// Keep every authored volume's identity/profile for query-hit resolution.
/// This does not choose a current volume or infer a water height from its model.
pub fn load_environment_volumes(
    document: &entities::Document,
    catalog: &archive::Catalog,
) -> Result<Vec<EnvironmentVolume>, String> {
    entities::placed_environment_volumes(document)
        .into_iter()
        .map(|volume| resolve(volume, catalog))
        .collect()
}

#[cfg(test)]
#[path = "volumes_tests.rs"]
mod tests;
