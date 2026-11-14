//! Compiled cloth companion header projection and standard skinned product selection.
//! 1356d10 registers arkCloth;088740 reads five model names before the Havok
//! payload.088c10/087820/087a20 rewrite MD6 mesh references before1c8cf0 loads
//! them. We retain only immutable header names: no cloth simulation or frozen
//! substitute is admitted by this resource-selection owner.
use crate::{archive::Catalog, assets::resource_registry::Registry};
use std::sync::{Arc, Mutex};

pub(crate) struct Header {
    products: [String; 5],
}
pub(crate) type Owner = Mutex<Registry<Header>>;
fn owner(catalog: &Catalog) -> &Owner {
    catalog.cloth_headers.get_or_init(Default::default)
}
fn normalize(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

fn acquire(
    catalog: &Catalog,
    path: &str,
    marker: u8,
    cancelled: Option<&dyn Fn() -> bool>,
) -> Result<Arc<Header>, String> {
    super::check_model_acquisition(cancelled)?;
    let path = normalize(path);
    let mut registry = owner(catalog)
        .lock()
        .map_err(|_| "cloth header registry is poisoned")?;
    let mut load = |_| {
        // Only the five names and mapping boundary are consumed. Use the
        // archive's streaming prefix reader, never decompress the cloth pack.
        let mut limit = 1024usize;
        let mut prefix_bytes = 0usize;
        loop {
            super::check_model_acquisition(cancelled)?;
            let bytes = catalog
                .read_prefix(&path, limit)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("{path}: compiled cloth header is absent"))?;
            prefix_bytes += bytes.len();
            crate::loading::read_catalog(catalog.identity(), "arkCloth", &path);
            match header_prefix(&bytes)? {
                HeaderPrefix::Complete(header) => {
                    super::check_model_acquisition(cancelled)?;
                    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                        eprintln!(
                            "CLOTH_HEADER_SOURCE path={path} prefix_bytes={prefix_bytes} payload_retained=false"
                        );
                    }
                    break Ok(Arc::new(header));
                }
                HeaderPrefix::Need(required) if bytes.len() == limit => limit = required,
                HeaderPrefix::Need(required) => {
                    return Err(format!(
                        "{path}: cloth header truncated (need {required} bytes, got {})",
                        bytes.len()
                    ));
                }
            }
        }
    };
    let result = if catalog.has_overlay_file(&path) {
        registry.acquire_with_sharing(&path, marker, false, &mut load)
    } else {
        registry.acquire_when(&path, marker, &mut load, || {
            super::check_model_acquisition(cancelled).is_ok()
        })
    }?;
    drop(registry);
    super::check_model_acquisition(cancelled)?;
    Ok(result)
}

enum HeaderPrefix {
    Complete(Header),
    Need(usize),
}

//088740 uses signed lengths: nonpositive values denote an empty product.
//The remaining40 bytes belong to the simulated-vertex mapping, not a sixth
//model name. That payload and the Havok pack are not retained here.
fn header_prefix(bytes: &[u8]) -> Result<HeaderPrefix, String> {
    let mut at = 0usize;
    let mut products: [String; 5] = Default::default();
    for product in &mut products {
        let end = at.checked_add(4).ok_or("cloth header offset overflow")?;
        let Some(length) = bytes.get(at..end) else {
            return Ok(HeaderPrefix::Need(end));
        };
        let length = i32::from_le_bytes(length.try_into().unwrap()).max(0) as usize;
        at = end;
        let end = at
            .checked_add(length)
            .ok_or("cloth product name offset overflow")?;
        let Some(name) = bytes.get(at..end) else {
            return Ok(HeaderPrefix::Need(end));
        };
        *product = std::str::from_utf8(name)
            .map_err(|error| error.to_string())?
            .trim_end_matches('\0')
            .to_owned();
        at = end;
    }
    let end = at
        .checked_add(40)
        .ok_or("cloth header mapping offset overflow")?;
    if end > bytes.len() {
        return Ok(HeaderPrefix::Need(end));
    }
    Ok(HeaderPrefix::Complete(Header { products }))
}

pub(crate) fn register_preload(catalog: &Catalog, path: &str, marker: u8) -> Result<(), String> {
    owner(catalog)
        .lock()
        .map_err(|_| "cloth header registry is poisoned")?
        .register(&normalize(path), marker, false);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.cloth_headers.get() {
        owner
            .lock()
            .map_err(|_| "cloth header registry is poisoned")?
            .promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.cloth_headers.get() {
        owner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .release(catalog.identity(), "arkCloth", marker);
    }
    report(catalog, "scope-released");
}
pub(crate) fn report(catalog: &Catalog, phase: &str) {
    if let Some(owner) = catalog.cloth_headers.get() {
        owner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .report_with(
                catalog.identity(),
                "arkCloth",
                phase,
                |header| header.products.iter().map(String::capacity).sum(),
                String::capacity,
            );
    }
}

//088500/1420c0 extracts the basename through its first extension separator.
fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .split('.')
        .next()
        .unwrap_or("")
}

/// None means this local mesh has no authored cloth declaration. Some is the
/// real optional noncloth product, not a filename heuristic or LOD fallback.
/// LOD0 is the current renderer's supported selection; other LOD companion
/// names are held by their actual header for future consumers.
pub(crate) fn select_base(
    catalog: &Catalog,
    declaration: &str,
    source: &str,
    mesh: &str,
    marker: u8,
    cancelled: Option<&dyn Fn() -> bool>,
) -> Result<Option<Option<String>>, String> {
    // Ordinary MD6 acquisitions need no second lexer pass.
    if !source.contains("cloth") {
        return Ok(None);
    }
    let tokens = crate::entities::declaration_lexemes(source)?;
    select_base_tokens(catalog, declaration, &tokens, mesh, marker, cancelled)
}

pub(crate) fn select_base_tokens(
    catalog: &Catalog,
    declaration: &str,
    tokens: &[&str],
    mesh: &str,
    marker: u8,
    cancelled: Option<&dyn Fn() -> bool>,
) -> Result<Option<Option<String>>, String> {
    let mut specs = Vec::new();
    let mut index = 0;
    while index + 2 < tokens.len() {
        if tokens[index] != "lod" || tokens[index + 1] != "0" || tokens[index + 2] != "{" {
            index += 1;
            continue;
        }
        index += 3;
        let mut depth = 1usize;
        while index < tokens.len() && depth != 0 {
            match tokens[index] {
                "{" => depth += 1,
                "}" => depth -= 1,
                "mesh" if depth == 1 && tokens.get(index + 1).copied() == Some(mesh) => {
                    index += 2;
                    if tokens.get(index).copied() != Some("{") {
                        continue;
                    }
                    index += 1;
                    while tokens.get(index).copied() != Some("}") {
                        if tokens.get(index).copied() != Some("cloth") {
                            return Err(format!("{declaration}: unsupported MD6 mesh token"));
                        }
                        let values = tokens
                            .get(index + 1..index + 6)
                            .ok_or("truncated MD6 cloth declaration")?;
                        let spec = (values[0], values[1]);
                        if !specs.contains(&spec) {
                            specs.push(spec);
                        }
                        index += 6;
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }
    if specs.is_empty() {
        return Ok(None);
    }
    let logical = declaration
        .strip_prefix("generated/decls/md6def/")
        .and_then(|path| path.strip_suffix(".decl"))
        .ok_or("cloth MD6 declaration has no logical source identity")?;
    //1c5d60 gives sixteen buckets;1c89c0 orders each by source/setup;
    //087820 traverses those buckets before088580 appends each LOD product.
    specs.sort_by_key(|&(file, setup)| {
        (
            (crate::entities::declaration_name_hash(file.as_bytes())
                ^ crate::entities::declaration_name_hash(setup.as_bytes()))
                & 15,
            file,
            setup,
        )
    });
    let mut hash_words = Vec::with_capacity((specs.len() + 1) * 4);
    hash_words.extend(crate::entities::declaration_name_hash(logical.as_bytes()).to_le_bytes());
    let mut companions = 0usize;
    for (file, setup) in specs {
        let path = format!(
            "generated/arkcloth/{}_{}_{}.bcloth",
            basename(logical),
            basename(file),
            setup
        );
        let header = acquire(catalog, &path, marker, cancelled)?;
        if !header.products[0].is_empty() {
            hash_words.extend(
                crate::entities::declaration_name_hash(header.products[0].as_bytes()).to_le_bytes(),
            );
            companions += 1;
        }
    }
    let digest = crate::assets::physworld::physics_system::md5(&hash_words);
    use std::fmt::Write;
    let mut signature = String::with_capacity(32);
    for byte in digest {
        write!(&mut signature, "{byte:02x}").expect("String formatting is infallible");
    }
    let path = format!("nocloth_{}_{signature}.bmd6model", basename(mesh));
    //087a20 uses existing-only lookup: an absent base removes the original
    //mesh. A cloth-only object has no supported ordinary skinned projection.
    if catalog.kind(&path).is_none() && !catalog.has_overlay_file(&path) {
        // The MD6 definition and its skeleton are still valid. Only an
        // ordinary mesh consumer must reject this cloth-only projection.
        return Ok(Some(None));
    }
    super::check_model_acquisition(cancelled)?;
    eprintln!(
        "MD6_CLOTH_SELECTION declaration={declaration} base={path} companions={companions} cloth_runtime=false"
    );
    Ok(Some(Some(path)))
}
