//! Static code-reference defaults: idDeclRenderParm constructor/destructor
//! 1402d82c0/1402d8310, parser1402d8820 and code rebinding1400606f0/14036f800.
//! Host: Catalog/session ownership replaces engine-global code references.
//! Keep original bytes, coalesce immutable acquisition/failure, and decode
//! only requested metadata. GPU uniforms/textures belong to their own owners.
use crate::archive::Catalog;
use std::{borrow::Cow, collections::HashMap, sync::{Arc, Mutex, OnceLock}};

#[derive(Default)]
pub(crate) struct Owner {
    entries: Mutex<HashMap<Arc<str>, Result<Arc<Parameter>, String>>>,
}
pub(crate) struct Parameter {
    path: Arc<str>,
    bytes: Vec<u8>,
    parsed: OnceLock<Box<Parsed>>,
}
#[derive(Default)]
struct Parsed {
    metadata: OnceLock<Result<crate::render_program::ParameterMetadata, String>>,
    floating: OnceLock<Result<[u32; 4], String>>,
}
impl Parameter {
    fn parsed(&self) -> &Parsed { self.parsed.get_or_init(|| Box::new(Parsed::default())) }
    pub(crate) fn bytes(&self) -> &[u8] { &self.bytes }
    pub(crate) fn text(&self) -> Cow<'_, str> { crate::entities::declaration_text(&self.bytes) }
    pub(crate) fn metadata(&self) -> Result<crate::render_program::ParameterMetadata, String> {
        self.parsed().metadata.get_or_init(|| parse_metadata(&self.path, &self.bytes)).clone()
    }
    /// Preserve ProgramLibrary's existing lossy-UTF8 initializer path. Other
    /// readers keep their existing native-byte text conversion via text().
    pub(crate) fn floating_words(&self) -> Result<[u32; 4], String> {
        self.parsed().floating.get_or_init(|| crate::entities::declaration_float_vector_default(
            &String::from_utf8_lossy(&self.bytes)).map_err(|error| format!("{}: {error}", self.path))).clone()
    }
}
pub(crate) fn acquire(catalog: &Catalog, name: &str) -> Result<Arc<Parameter>, String> {
    let _tag = crate::platform::allocation_tags::tag("Render parameter resources");
    let path = format!("generated/decls/renderparm/{}.decl", name.replace('\\', "/").to_ascii_lowercase());
    let shared = !catalog.has_overlay_file(&path)
        && std::env::var("DH2_DEBUG_SEPARATE_RENDER_PARAMETERS").as_deref() != Ok("1");
    let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
    let mut read = false;
    let mut load = |path: Arc<str>| {
        read = true;
        let bytes = catalog.read(&path).map_err(|error| format!("{path}: {error}"))?
            .ok_or_else(|| format!("missing render-parameter declaration {path}"))?;
        Ok(Arc::new(Parameter { path, bytes, parsed: OnceLock::new() }))
    };
    let result = if shared {
        let mut entries = catalog.render_parameters.get_or_init(Owner::default).entries.lock()
            .map_err(|_| "render parameter owner is poisoned")?;
        if let Some(value) = entries.get(path.as_str()) { value.clone() }
        else {
            let path: Arc<str> = path.into();
            let result = load(path.clone());
            entries.insert(path, result.clone());
            result
        }
    } else { load(path.into()) };
    if trace {
        let mut crc = flate2::Crc::new();
        if let Ok(value) = &result { crc.update(&value.bytes); }
        eprintln!("RENDER_PARAMETER_ACQUIRE name={name} read={read} shared={shared} signature={:08x} success={}", crc.sum(), result.is_ok());
    }
    result
}
pub(crate) fn report(catalog: &Catalog, phase: &str) {
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() != Ok("1") { return; }
    if let Some(owner) = catalog.render_parameters.get() {
        let entries = owner.entries.lock().unwrap_or_else(|error| error.into_inner());
        let mut source_bytes = 0; let mut source_capacity = 0; let mut metadata = 0; let mut floating = 0;
        // Requested storage on this host, excluding hash control bytes and
        // allocator rounding. Node and key share each Arc<str> allocation.
        let arc_node = std::alloc::Layout::new::<[usize; 2]>()
            .extend(std::alloc::Layout::new::<Parameter>()).expect("parameter Arc layout")
            .0.pad_to_align().size();
        let mut requested_bytes = entries.capacity()
            * std::mem::size_of::<(Arc<str>, Result<Arc<Parameter>, String>)>();
        for (path, value) in entries.iter() {
            requested_bytes += (2 * std::mem::size_of::<usize>() + path.len())
                .next_multiple_of(std::mem::align_of::<usize>());
            match value {
                Err(error) => requested_bytes += error.capacity(),
                Ok(value) => {
                    source_bytes += value.bytes.len(); source_capacity += value.bytes.capacity();
                    requested_bytes += arc_node + value.bytes.capacity();
                    if let Some(parsed) = value.parsed.get() {
                        requested_bytes += std::mem::size_of::<Parsed>();
                        metadata += usize::from(parsed.metadata.get().is_some());
                        floating += usize::from(parsed.floating.get().is_some());
                        if let Some(Err(error)) = parsed.metadata.get() { requested_bytes += error.capacity(); }
                        if let Some(Err(error)) = parsed.floating.get() { requested_bytes += error.capacity(); }
                    }
                }
            }
        }
        eprintln!("RENDER_PARAMETER_STORAGE catalog={} phase={phase} records={} source_bytes={source_bytes} source_capacity={source_capacity} requested_bytes={requested_bytes} metadata={metadata} floating={floating}", catalog.identity(), entries.len());
    }
}

fn parse_metadata(path: &str, bytes: &[u8]) -> Result<crate::render_program::ParameterMetadata, String> {
    use crate::render_program::{NumericStorage, ParameterMetadata, RenderParameterKind};
    let text = crate::entities::declaration_text(bytes);
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
