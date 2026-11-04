//! Immutable render-parameter metadata shared by the session's program owners.
//! The original resource registry retains initialized declarations separately
//! from each world's prepared render programs (1400607f0/14005feb0).
use super::{Arc, HashMap, LazyLock, Mutex, Path, Result};
use crate::{assets::archive::Catalog, rendering::render_program::RenderParameterKind};

#[derive(Clone)]
pub(super) struct Parameters {
    pub kinds: Arc<HashMap<Vec<u8>, RenderParameterKind>>,
    pub options: Arc<HashMap<Vec<u8>, i32>>,
}

static RESIDENT: LazyLock<Mutex<Option<(u64, std::path::PathBuf, Parameters)>>> = LazyLock::new(Default::default);

pub(super) fn acquire(base: &Path, catalog: &Catalog) -> Result<Parameters> {
    let started = std::time::Instant::now();
    let mut resident = RESIDENT.lock().unwrap_or_else(|error| error.into_inner());
    let shared = std::env::var("DH2_DEBUG_UNSHARED_SHADER_METADATA").as_deref() != Ok("1");
    let (parameters, reused) = if let Some((_, _, parameters)) = resident.as_ref().filter(|(identity, path, _)| *identity == catalog.identity() && path == base && shared) {
        (parameters.clone(), true)
    } else {
        if shared { *resident = None; }
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
        if shared { *resident = Some((catalog.identity(), base.to_owned(), parameters.clone())); }
        (parameters, false)
    };
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("MEMORY shader parameter acquisition: reused={reused} kinds={} options={} identity={:p} acquire_ms={:.3}",
            parameters.kinds.len(), parameters.options.len(), Arc::as_ptr(&parameters.kinds), started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(parameters)
}
