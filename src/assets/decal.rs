//! Installed decal declaration's four receiver/instancing material references.
use super::{
    archive::Catalog,
    entities::{declaration_text, parse_declaration},
};
use crate::gameplay::options::decl::{atom, scope};
use std::sync::Arc;

/// Acquisition facade; Catalog owns immutable identities and scope markers.
#[derive(Default)]
pub struct DecalResources;

#[derive(Clone)]
pub(crate) enum ResourceError {
    Missing,
    Invalid(String),
}
impl DecalResources {
    pub fn resolve(&mut self, catalog: &Catalog, name: &str)
        -> Result<Option<Arc<DecalMaterials<String>>>, String> {
        let name = name.replace(char::from(92), "/").to_ascii_lowercase();
        let path = format!("generated/decls/decal/{name}.decl");
        let shared = !catalog.has_overlays();
        let value = catalog.decal_resources.get_or_init(Default::default).lock()
            .map_err(|_| "decal resource registry is poisoned")?
            .acquire_with_sharing(&path, 0, shared, |_| {
                let material = DecalMaterials::read(catalog, &name)
                    .map_err(ResourceError::Invalid)?.ok_or(ResourceError::Missing)?;
                material.resolve(catalog).map(Arc::new).map_err(ResourceError::Invalid)
            });
        match value {
            Ok(material) => Ok(Some(material)),
            Err(ResourceError::Missing) => Ok(None),
            Err(ResourceError::Invalid(error)) => Err(error),
        }
    }
}

pub(crate) fn register_preload(catalog: &Catalog, path: &str, marker: u8, persistent: bool) -> Result<(), String> {
    catalog.decal_resources.get_or_init(Default::default).lock()
        .map_err(|_| "decal resource registry is poisoned")?.register(path, marker, persistent);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.decal_resources.get() {
        owner.lock().map_err(|_| "decal resource registry is poisoned")?.promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.decal_resources.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.release(catalog.identity(), "decal", marker);
        report(&owner, catalog, "scope-released");
    }
}
pub(crate) fn report_retirement(catalog: &Catalog) {
    if let Some(owner) = catalog.decal_resources.get() {
        report(&owner.lock().unwrap_or_else(|error| error.into_inner()), catalog, "catalog-retiring");
    }
}
fn report(owner: &crate::assets::resource_registry::Registry<DecalMaterials<String>, ResourceError>,
    catalog: &Catalog, phase: &str) {
    owner.report_with(catalog.identity(), "decal", phase, |material|
        [&material.opaque, &material.opaque_instanced, &material.alpha, &material.alpha_instanced]
            .into_iter().flatten().map(String::capacity).sum(),
        |error| match error { ResourceError::Missing => 0, ResourceError::Invalid(error) => error.capacity() });
}

/// The fields at 1414fc470. A missing variant is not replaced by another
/// receiver's shader: their geometry and blend/depth requirements differ.
#[derive(Clone, Debug)]
pub struct DecalMaterials<M> {
    pub opaque: Option<M>,
    pub opaque_instanced: Option<M>,
    pub alpha: Option<M>,
    pub alpha_instanced: Option<M>,
}
impl<M> AsRef<DecalMaterials<M>> for DecalMaterials<M> {
    fn as_ref(&self) -> &Self { self }
}
impl<M> DecalMaterials<M> {
    pub fn select(&self, opaque: bool, instanced: bool) -> Option<&M> {
        match (opaque, instanced) {
            (true, false) => self.opaque.as_ref(),
            (true, true) => self.opaque_instanced.as_ref(),
            (false, false) => self.alpha.as_ref(),
            (false, true) => self.alpha_instanced.as_ref(),
        }
    }

    pub fn try_map<N, E>(
        self,
        mut load: impl FnMut(M) -> Result<N, E>,
    ) -> Result<DecalMaterials<N>, E> {
        Ok(DecalMaterials {
            opaque: self.opaque.map(&mut load).transpose()?,
            opaque_instanced: self.opaque_instanced.map(&mut load).transpose()?,
            alpha: self.alpha.map(&mut load).transpose()?,
            alpha_instanced: self.alpha_instanced.map(&mut load).transpose()?,
        })
    }
}
impl DecalMaterials<String> {
    /// Read directly from the installed catalog. Names are those stored in the
    /// declaration, including suffixless references; resolution is separate.
    pub fn read(catalog: &Catalog, name: &str) -> Result<Option<Self>, String> {
        let resource = format!("generated/decls/decal/{name}.decl");
        let Some(bytes) = catalog
            .read(&resource)
            .map_err(|e| format!("{resource}: {e}"))?
        else {
            return Ok(None);
        };
        let fields = parse_declaration(&declaration_text(&bytes))?;
        let edit =
            scope(&fields, "edit").ok_or_else(|| format!("{resource}: missing edit scope"))?;
        let reference = |field| {
            atom(edit, field)
                .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
                .map(str::to_owned)
        };
        Ok(Some(Self {
            opaque: reference("m_materialOnOpaque"),
            opaque_instanced: reference("m_materialOnOpaqueWithMultiInstancing"),
            alpha: reference("m_materialOnAlpha"),
            alpha_instanced: reference("m_materialOnAlphaWithMultiInstancing"),
        }))
    }

    /// Installed decal declarations contain both suffixed and suffixless
    /// material references. Accept a spelling only when its resource exists.
    /// Never substitute a different variant when the selected asset is absent.
    pub fn resolve(self, catalog: &Catalog) -> Result<Self, String> {
        self.try_map(|name| {
            if catalog.contains(&format!("generated/decls/material/{name}.decl")) {
                return Ok(name);
            }
            let suffixed = format!("{name}.material");
            if !name.to_ascii_lowercase().ends_with(".material")
                && catalog.contains(&format!("generated/decls/material/{suffixed}.decl"))
            {
                return Ok(suffixed);
            }
            Err(format!("decal material resource absent: {name}"))
        })
    }
}


/// Observe the installed declarations through independent acquisition facades.
#[cfg(feature = "game")]
pub(crate) fn observe_resources(base: &std::path::Path) -> Result<(), String> {
    let a = Catalog::open(base).map_err(|error| error.to_string())?;
    let b = Catalog::open(base).map_err(|error| error.to_string())?;
    let paths = a.paths_with_suffix(".decl").into_iter()
        .filter(|path| a.kind(path) == Some("decal")).collect::<Vec<_>>();
    if paths.is_empty() { return Err("installed decal declarations are absent".into()); }
    let mut one = DecalResources;
    let mut two = DecalResources;
    let mut references = 0;
    for path in &paths {
        let name = path.strip_prefix("generated/decls/decal/").and_then(|name| name.strip_suffix(".decl"))
            .ok_or("invalid installed decal path")?;
        let reference = DecalMaterials::read(&a, name)?.ok_or("installed decal missing")?.resolve(&a)?;
        let first = one.resolve(&a, name)?.ok_or("first decal acquisition missing")?;
        let again = two.resolve(&a, name)?.ok_or("shared decal acquisition missing")?;
        let other = two.resolve(&b, name)?.ok_or("independent decal acquisition missing")?;
        let signature = |value: &DecalMaterials<String>| blake3::hash(format!("{value:?}").as_bytes());
        if !Arc::ptr_eq(&first, &again) || Arc::ptr_eq(&first, &other)
            || signature(&first) != signature(&reference) || signature(&other) != signature(&reference) {
            return Err(format!("decal values or identities differ: {path}"));
        }
        references += [&reference.opaque, &reference.opaque_instanced, &reference.alpha, &reference.alpha_instanced]
            .into_iter().flatten().count();
        let weak = Arc::downgrade(&first);
        a.release_level_resources();
        if !crate::loading::catalog_read_recorded(a.identity(), "decal", path) {
            return Err("live decal lost its read credit".into());
        }
        drop(first);
        drop(again);
        a.release_level_resources();
        if weak.strong_count() != 0 || crate::loading::catalog_read_recorded(a.identity(), "decal", path)
            || !crate::loading::catalog_read_recorded(b.identity(), "decal", path) {
            return Err("decal retirement crossed catalog or consumer boundary".into());
        }
        drop(other);
        b.release_level_resources();
        if crate::loading::catalog_read_recorded(b.identity(), "decal", path) {
            return Err("second catalog retained a dead decal".into());
        }
        eprintln!("DECAL_RESOURCE_CONTENT path={path} signature={} independent=true retired=true", signature(&reference));
    }
    let path = &paths[0];
    let name = path.strip_prefix("generated/decls/decal/").and_then(|name| name.strip_suffix(".decl"))
        .ok_or("invalid bootstrap decal path")?;
    register_preload(&a, path, 0, false)?;
    promote_bootstrap(&a)?;
    let value = one.resolve(&a, name)?.ok_or("bootstrap decal missing")?;
    let weak = Arc::downgrade(&value);
    drop(value);
    a.release_level_resources();
    a.release_persistent_level_resources();
    let again = two.resolve(&a, name)?.ok_or("bootstrap decal missing after purge")?;
    if !std::ptr::addr_eq(weak.as_ptr(), Arc::as_ptr(&again)) {
        return Err("ordinary purges replaced a bootstrap decal".into());
    }
    register_preload(&a, path, 0, true)?;
    drop(again);
    a.release_level_resources();
    if weak.strong_count() == 0 { return Err("map purge released a heap-1 decal".into()); }
    a.release_persistent_level_resources();
    if weak.strong_count() != 0 || crate::loading::catalog_read_recorded(a.identity(), "decal", path) {
        return Err("persistent purge retained the decal".into());
    }
    let final_value = one.resolve(&a, name)?.ok_or("final decal acquisition missing")?;
    let weak = Arc::downgrade(&final_value);
    drop(a);
    if weak.strong_count() != 1 { return Err("catalog retained or invalidated the final decal consumer".into()); }
    drop(final_value);
    if weak.strong_count() != 0 { return Err("final decal consumer did not retire".into()); }
    eprintln!("DECAL_RESOURCE_SCOPES definitions={} references={references} bootstrap=true persistent=true independent=true final_release=true", paths.len());
    Ok(())
}
