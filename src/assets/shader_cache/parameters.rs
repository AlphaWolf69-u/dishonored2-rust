//! Immutable render-parameter metadata shared by the session's program owners.
//! The original resource registry retains initialized declarations separately
//! from each world's prepared render programs (1400607f0/14005feb0).
use super::{Arc, HashMap, Result};
use crate::{assets::archive::Catalog, rendering::render_program::RenderParameterKind};

#[derive(Clone)]
pub(crate) struct Parameters {
    pub kinds: Arc<HashMap<Vec<u8>, RenderParameterKind>>,
    pub options: Arc<HashMap<Vec<u8>, i32>>,
}

/// 1402d8820 registers the numeric union subtype and color annotation used
/// by 14098cdd0. Parse only requested parameter-block metadata.
pub(super) fn value_metadata(catalog: &Catalog, name: &str) -> Result<crate::render_program::ParameterMetadata> {
    crate::assets::render_parameter::acquire(catalog, name)?.metadata()
}

pub(super) fn acquire(catalog: &Catalog) -> Result<Parameters> {
    let started = std::time::Instant::now();
    // A loose declaration can change while the catalog identity remains the same.
    let shared = !catalog.has_overlay_declaration("generated/decls/renderparm/")
        && std::env::var("DH2_DEBUG_UNSHARED_SHADER_METADATA").as_deref() != Ok("1");
    let mut resident = shared.then(|| {
        catalog.render_parameters.get_or_init(Default::default).shader_metadata
            .lock().unwrap_or_else(|error| error.into_inner())
    });
    let (parameters, reused) = if let Some(parameters) = resident.as_ref().and_then(|slot| slot.as_ref()) {
        (parameters.clone(), true)
    } else {
        let mut kinds = HashMap::new();
        let mut options = HashMap::new();
        crate::render_program::visit_render_parameters(catalog, |name, kind, source| {
            if kind == RenderParameterKind::Option {
                options.insert(name.to_vec(), crate::entities::declaration_option_default(source)?);
            }
            kinds.insert(name.to_vec(), kind);
            Ok(())
        })?;
        let parameters = Parameters { kinds: Arc::new(kinds), options: Arc::new(options) };
        if let Some(slot) = resident.as_mut() { **slot = Some(parameters.clone()); }
        (parameters, false)
    };
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("MEMORY shader parameter acquisition: reused={reused} kinds={} options={} identity={:p} acquire_ms={:.3}",
            parameters.kinds.len(), parameters.options.len(), Arc::as_ptr(&parameters.kinds), started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(parameters)
}
