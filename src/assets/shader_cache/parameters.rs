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

pub(crate) fn retire_catalog(catalog: u64) -> usize {
    let retired = RESIDENT.lock().unwrap_or_else(|e| e.into_inner())
        .take_if(|(identity, _, _)| *identity == catalog);
    usize::from(retired.is_some())
}

/// 1402d8820 registers the numeric union subtype and color annotation used
/// by 14098cdd0. Parse only requested parameter-block metadata.
pub(super) fn value_metadata(catalog: &Catalog, name: &str) -> Result<crate::render_program::ParameterMetadata> {
    use crate::render_program::{NumericStorage, ParameterMetadata, RenderParameterKind};
    let path = format!("generated/decls/renderparm/{name}.decl");
    let bytes = catalog.read(&path).map_err(|error| error.to_string())?
        .ok_or_else(|| format!("renderParm absent: {path}"))?;
    let text = crate::entities::declaration_text(&bytes);
    let kind = crate::render_program::render_parameter_kind(&text)?;
    let tokens = crate::entities::declaration_lexemes(&text)?;
    let (numeric, lanes) = match kind {
        RenderParameterKind::Option => (Some(NumericStorage::Unsigned), 1),
        RenderParameterKind::Uniform | RenderParameterKind::Semantic => {
            if tokens.iter().any(|token| token.eq_ignore_ascii_case("color")) {
                (Some(NumericStorage::Color), 4)
            } else if tokens.get(1).is_some_and(|token| token.eq_ignore_ascii_case("vec")) {
                (Some(NumericStorage::Float), 4)
            } else {
                let subtype = tokens.iter().skip(2).find(|token| !token.eq_ignore_ascii_case("notbakeable"))
                    .ok_or_else(|| format!("renderParm has no numeric type: {path}"))?;
                let lower = subtype.to_ascii_lowercase();
                let storage = match lower.as_str() {
                    "float" | "float2" | "float3" | "float4" => NumericStorage::Float,
                    "int" | "int2" | "int3" | "int4" => NumericStorage::Signed,
                    "uint" | "uint2" | "uint3" | "uint4" | "bool" => NumericStorage::Unsigned,
                    _ => return Err(format!("unsupported renderParm numeric type {subtype:?}: {path}")),
                };
                let lanes = match lower.as_bytes().last() { Some(b'2') => 2, Some(b'3') => 3, Some(b'4') => 4, _ => 1 };
                (Some(storage), lanes)
            }
        }
        _ => (None, 0),
    };
    Ok(ParameterMetadata { kind, numeric, lanes })
}

pub(super) fn acquire(base: &Path, catalog: &Catalog) -> Result<Parameters> {
    let started = std::time::Instant::now();
    let mut resident = RESIDENT.lock().unwrap_or_else(|error| error.into_inner());
    // A loose declaration can change while the catalog identity remains the same.
    let shared = !catalog.has_overlay_declaration("generated/decls/renderparm/")
        && std::env::var("DH2_DEBUG_UNSHARED_SHADER_METADATA").as_deref() != Ok("1");
    if resident.as_ref().is_some_and(|(identity, path, _)| *identity != catalog.identity() || path != base) {
        *resident = None;
    }
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
