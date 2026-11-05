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

/// The live physics and water owners need only these selected values.
pub(crate) struct RuntimeEnvironmentVolume {
    pub id: String,
    pub position: [f32; 3],
    pub is_water_volume: bool,
    pub surface_material: Option<String>,
    // Diagnostic comparison keeps the preparation payload alive.
    pub _retained: Option<Box<EnvironmentVolume>>,
}

pub(super) fn resolve(
    placement: entities::PlacedEnvironmentVolume,
    catalog: &archive::Catalog,
) -> Result<EnvironmentVolume, String> {
    resolve_with_cache(placement, catalog, None)
}

fn resolve_with_cache(
    placement: entities::PlacedEnvironmentVolume,
    catalog: &archive::Catalog,
    cache: Option<&mut entities::EnvironmentParentCache>,
) -> Result<EnvironmentVolume, String> {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let trace = *TRACE.get_or_init(|| std::env::var("DH2_DEBUG_ENVIRONMENT_ACQUISITIONS").as_deref() == Ok("1"));
    let mut reads = 0usize;
    let mut decoded_bytes = 0usize;
    let parameters = entities::environment_parameters_with_parent_cache(
        &placement.id,
        &placement.environment_decl,
        |parent| {
            let bytes = catalog
                .read(&format!("generated/decls/env/{parent}.decl"))
                .map_err(|e| e.to_string())?;
            if trace { reads += 1; decoded_bytes += bytes.as_ref().map_or(0, Vec::len); }
            Ok(bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
        }, cache,
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
    if trace {
        let mut crc = flate2::Crc::new();
        crc.update(format!("{parameters:?}{is_water_volume:?}{surface_material:?}").as_bytes());
        eprintln!("MEMORY environment acquisition: id={} reads={reads} decoded_bytes={decoded_bytes} signature={:08x}", placement.id, crc.sum());
        if std::env::var("DH2_DEBUG_ENVIRONMENT_LAYERS").as_deref() == Ok("1") {
            for layer in &parameters.layers {
                let mut crc = flate2::Crc::new();
                crc.update(format!("{:?}", layer.parameters).as_bytes());
                let capacity = layer.resource.capacity() + layer.parameters.capacity() * std::mem::size_of::<(String, String)>()
                    + layer.parameters.iter().map(|(key, value)| key.capacity() + value.capacity()).sum::<usize>();
                eprintln!("MEMORY environment layer: resource={} capacity={capacity} signature={:08x}", layer.resource, crc.sum());
            }
        }
    }
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
    let shared = !catalog.has_overlays()
        && std::env::var("DH2_DEBUG_SEPARATE_ENVIRONMENT_PROFILES").as_deref() != Ok("1")
        && std::env::var("DH2_DEBUG_SEPARATE_ENVIRONMENT_PARENTS").as_deref() != Ok("1");
    let mut cache = entities::EnvironmentParentCache::default();
    let result = entities::placed_environment_volumes(document)
        .into_iter()
        .map(|volume| resolve_with_cache(volume, catalog, shared.then_some(&mut cache)))
        .collect();
    if shared { cache.report(); }
    result
}

#[cfg(test)]
#[path = "volumes_tests.rs"]
mod tests;
