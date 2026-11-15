//! arkDeclFade (colour at +0x58, opacity +0xd0), sampled by 140167170.
use super::*;
use crate::gameplay::effects::{curve::EffectFloatCurve, vector_curve::{EffectVectorCurve, EffectVectorCurvePair}};
pub struct Fade {
    pub colour: EffectVectorCurvePair<3>,
    pub opacity: EffectFloatCurve,
}
impl Default for Fade {
    fn default() -> Self {
        // 00f610 -> 00f740 resets RGB and opacity and appends one t=0 key.
        // Both constructors (00d610 / 00d0b0) select interpolation mode 4,
        // hence [incoming, value, outgoing] storage even for that single key.
        // Partial edits without `num` modify this constructor-owned storage.
        Self {
            colour: EffectVectorCurvePair {
                maximum: EffectVectorCurve { times: vec![0.], values: vec![[0.; 3]; 3] },
                minimum: Default::default(),
            },
            opacity: EffectFloatCurve { times: vec![0.], values: vec![0.; 3] },
        }
    }
}
impl Fade {
    pub fn parse(text: &str) -> Result<Self, String> {
        let fields = crate::entities::parse_declaration(text).map_err(|e| e.to_string())?;
        let edit = fields
            .iter()
            .find(|(name, _)| name == "edit")
            .map(|(_, v)| scope(v))
            .transpose()?
            .unwrap_or(&fields);
        let mut result = Self::default();
        if let Some((_, fields)) = edit.iter().find(|(name, _)| name == "m_fadeCurves") {
            for (name, value) in scope(fields)? {
                match name.as_str() {
                    "m_color" => {
                        particles::vector_curve(&mut result.colour.maximum, scope(value)?)?
                    }
                    "m_opacity" => curve(&mut result.opacity, scope(value)?)?,
                    _ => return Err(format!("unknown fade curve {name}")),
                }
            }
        }
        Ok(result)
    }
    pub fn rgba(&self, time: f32) -> Result<[f32; 4], String> {
        let rgb = self.colour.sample(time, [0.; 3])?;
        let opacity = self.opacity.sample_visibility(time)?;
        Ok([
            rgb[0].clamp(0., 1.),
            rgb[1].clamp(0., 1.),
            rgb[2].clamp(0., 1.),
            opacity.clamp(0., 1.),
        ])
    }
}


/// Shared declaration identities (141351cd0, 1400607f0). The loading
/// controller owns its clock; this owner retains only immutable curves.
pub(crate) fn acquire(catalog: &crate::archive::Catalog, path: &str) -> Result<std::sync::Arc<Fade>, String> {
    let path = path.replace(char::from(92), "/").to_ascii_lowercase();
    let shared = !catalog.has_overlays()
        && std::env::var_os("DH2_DEBUG_SEPARATE_FADES").is_none_or(|value| value != "1");
    catalog.fade_resources.get_or_init(Default::default).lock()
        .map_err(|_| "fade resource registry is poisoned")?
        .acquire_with_sharing(&path, 0, shared, |_| {
            let bytes = catalog.read(&path).map_err(|error| error.to_string())?
                .ok_or_else(|| format!("missing fade {path}"))?;
            Fade::parse(&crate::entities::owned_declaration_text(bytes)).map(std::sync::Arc::new)
        })
}

pub(crate) fn register_preload(catalog: &crate::archive::Catalog, path: &str,
    marker: u8, persistent: bool) -> Result<(), String> {
    catalog.fade_resources.get_or_init(Default::default).lock()
        .map_err(|_| "fade resource registry is poisoned")?.register(path, marker, persistent);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &crate::archive::Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.fade_resources.get() {
        owner.lock().map_err(|_| "fade resource registry is poisoned")?.promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &crate::archive::Catalog, marker: u8) {
    if let Some(owner) = catalog.fade_resources.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.release(catalog.identity(), "fade", marker);
        owner.report(catalog.identity(), "fade", "scope-released", nested_bytes);
    }
}
pub(crate) fn report_retirement(catalog: &crate::archive::Catalog) {
    if let Some(owner) = catalog.fade_resources.get() {
        owner.lock().unwrap_or_else(|error| error.into_inner())
            .report(catalog.identity(), "fade", "catalog-retiring", nested_bytes);
    }
}
fn nested_bytes(fade: &Fade) -> usize {
    let vector = |curve: &EffectVectorCurve<3>| curve.times.capacity() * std::mem::size_of::<f32>()
        + curve.values.capacity() * std::mem::size_of::<[f32; 3]>();
    vector(&fade.colour.maximum) + vector(&fade.colour.minimum)
        + (fade.opacity.times.capacity() + fade.opacity.values.capacity()) * std::mem::size_of::<f32>()
}


/// Windowless observation of installed payloads and the owner's scope phases.
#[cfg(feature = "game")]
pub(crate) fn observe_resources(base: &std::path::Path) -> Result<(), String> {
    use crate::archive::Catalog;
    use std::sync::Arc;
    fn signature(value: &Fade) -> blake3::Hash {
        let mut digest = blake3::Hasher::new();
        for values in [&value.colour.maximum.times, &value.colour.minimum.times,
            &value.opacity.times, &value.opacity.values] {
            digest.update(&values.len().to_le_bytes());
            for value in values { digest.update(&value.to_bits().to_le_bytes()); }
        }
        for values in [&value.colour.maximum.values, &value.colour.minimum.values] {
            digest.update(&values.len().to_le_bytes());
            for value in values.iter().flatten() { digest.update(&value.to_bits().to_le_bytes()); }
        }
        digest.finalize()
    }
    let a = Catalog::open(base).map_err(|error| error.to_string())?;
    let b = Catalog::open(base).map_err(|error| error.to_string())?;
    let paths = a.paths_with_suffix(".decl").into_iter()
        .filter(|path| a.kind(path) == Some("fade")).collect::<Vec<_>>();
    if paths.is_empty() { return Err("installed fades are absent".into()); }
    for path in &paths {
        let source = a.read(path).map_err(|error| error.to_string())?.ok_or("installed fade missing")?;
        let reference = Fade::parse(&crate::entities::owned_declaration_text(source))?;
        let first = acquire(&a, path)?;
        let again = acquire(&a, path)?;
        let other = acquire(&b, path)?;
        if !Arc::ptr_eq(&first, &again) || Arc::ptr_eq(&first, &other)
            || signature(&first) != signature(&reference) || signature(&other) != signature(&reference) {
            return Err(format!("fade content or catalog identity differs: {path}"));
        }
        let weak = Arc::downgrade(&first);
        a.release_level_resources();
        if !crate::loading::catalog_read_recorded(a.identity(), "fade", path) {
            return Err(format!("live fade lost its read credit: {path}"));
        }
        drop(first);
        drop(again);
        a.release_level_resources();
        if weak.strong_count() != 0 || crate::loading::catalog_read_recorded(a.identity(), "fade", path)
            || !crate::loading::catalog_read_recorded(b.identity(), "fade", path) {
            return Err(format!("fade retirement crossed catalog or consumer boundary: {path}"));
        }
        drop(other);
        b.release_level_resources();
        if crate::loading::catalog_read_recorded(b.identity(), "fade", path) {
            return Err(format!("second catalog retained a dead fade: {path}"));
        }
        eprintln!("FADE_RESOURCE_CONTENT path={path} signature={} independent=true retired=true", signature(&reference));
    }
    let path = &paths[0];
    register_preload(&a, path, 0, false)?;
    promote_bootstrap(&a)?;
    let bootstrap = acquire(&a, path)?;
    let weak = Arc::downgrade(&bootstrap);
    drop(bootstrap);
    a.release_level_resources();
    a.release_persistent_level_resources();
    let again = acquire(&a, path)?;
    if !std::ptr::addr_eq(weak.as_ptr(), Arc::as_ptr(&again)) {
        return Err("ordinary purges replaced the bootstrap fade".into());
    }
    // Explicit map preload overwrites even a promoted header for heap-1 kinds.
    register_preload(&a, path, 0, true)?;
    drop(again);
    a.release_level_resources();
    if weak.strong_count() == 0 { return Err("map purge released a heap-1 fade".into()); }
    a.release_persistent_level_resources();
    if weak.strong_count() != 0 || crate::loading::catalog_read_recorded(a.identity(), "fade", path) {
        return Err("heap-1 fade survived persistent purge".into());
    }
    let final_value = acquire(&a, path)?;
    let final_weak = Arc::downgrade(&final_value);
    drop(a);
    if final_weak.strong_count() != 1 { return Err("catalog retained or invalidated final consumer".into()); }
    drop(final_value);
    if final_weak.strong_count() != 0 { return Err("final fade consumer did not retire".into()); }
    eprintln!("FADE_RESOURCE_SCOPES definitions={} bootstrap=true persistent=true independent=true final_release=true", paths.len());
    Ok(())
}
