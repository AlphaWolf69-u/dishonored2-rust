//! Share GPU materials when their world/model routes agree; keep distinct
//! variants when the two source selectors assign different passes.
use super::{CachedMaterial, HashMap};

fn trace_source() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1"))
}
fn separate_source() -> bool {
    static SEPARATE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SEPARATE.get_or_init(|| std::env::var("DH2_DEBUG_SEPARATE_MATERIAL_SOURCE").as_deref() == Ok("1"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::app) enum MaterialGeometry {
    World,
    Model,
}

impl MaterialGeometry {
    fn index(self) -> usize {
        match self {
            Self::World => 0,
            Self::Model => 1,
        }
    }
}

#[derive(Default)]
pub(in crate::app) struct MaterialCache {
    // Transient canonical cooker mode; no image slots or ordinary cached
    // binding entries are produced while compiling preload program inputs.
    pipeline_only: bool,
    entries: HashMap<String, [Option<Option<CachedMaterial>>; 2]>,
    dependency_trace: Option<std::sync::Mutex<super::HashSet<(String, MaterialGeometry)>>>,
    image_priorities: Vec<u8>,
    pending_source: Option<(String, Vec<u8>)>,
    source_matches: usize,
    source_bytes: usize,
    max_source_capacity: usize,
    source_crc: Option<flate2::Crc>,
    admission_crc: Option<flate2::Crc>,
    resolved_crc: Option<flate2::Crc>,
    body_physics: HashMap<String, Option<std::sync::Arc<str>>>,
}

impl MaterialCache {
    pub(super) fn pipeline_only(&self) -> bool { self.pipeline_only }
    pub(super) fn set_pipeline_only(&mut self, value: bool) -> bool {
        std::mem::replace(&mut self.pipeline_only, value)
    }
    /// Borrow the physics declaration captured by the actual render-material
    /// owner. A missing cache entry or rejected render material is an error;
    /// only an admitted material with no authored reference yields `None`.
    pub(in crate::app) fn physics_material(&self, path: &str) -> Result<Option<&str>, String> {
        self.trace_dependency(path, MaterialGeometry::Model);
        let entries = self.entries.get(path)
            .ok_or_else(|| format!("render material was not resolved: {path}"))?;
        // Route compatibility affects the GPU material, not the declaration
        // identity. A world-first material supplies the same authored value.
        let cached = entries.iter().rev().filter_map(Option::as_ref)
            .find_map(Option::as_ref)
            .ok_or_else(|| format!("render material was not admitted: {path}"))?;
        if !cached.physics_material_source_present {
            return Err(format!("render material declaration is missing: {path}"));
        }
        if cached.physics_material_preprocessing_required {
            return Err(format!("render material physics reference requires unsupported lexer processing or scope: {path}"));
        }
        Ok(cached.physics_material.as_deref())
    }

    /// 140807c80 reads the material's physics declaration independently of
    /// whether this renderer emitted a surface. Reuse the existing admitted
    /// material when present; otherwise resolve its declaration through the
    /// same catalog and material owner. The literal NULL fallback is not yet
    /// established and must remain explicit.
    pub(in crate::app) fn body_physics_material(&mut self, catalog: &crate::archive::Catalog,
        path: &str) -> Result<Option<std::sync::Arc<str>>, String> {
        if let Ok(value) = self.physics_material(path) {
            return Ok(value.map(std::sync::Arc::from));
        }
        if crate::rendering::material::is_null_material(path) {
            return Err("native default material for NULL model surface is not resolved".into());
        }
        if !catalog.has_overlays() {
            if let Some(value) = self.body_physics.get(path) { return Ok(value.clone()); }
        }
        let resource = format!("generated/decls/material/{path}.decl");
        let bytes = catalog.read(&resource).map_err(|error| error.to_string())?
            .ok_or_else(|| format!("render material declaration is missing: {path}"))?;
        let surface = crate::rendering::material_surface::Surface::parse(&bytes);
        if surface.physics_material_preprocessing_required {
            return Err(format!("render material physics reference requires unsupported lexer processing or scope: {path}"));
        }
        let value = surface.physics_material.as_deref().map(std::sync::Arc::from);
        if !catalog.has_overlays() { self.body_physics.insert(path.to_owned(), value.clone()); }
        Ok(value)
    }

    pub(in crate::app) fn finish_body_physics_preparation(&mut self) {
        self.body_physics.clear();
        self.body_physics.shrink_to_fit();
    }

    // Transient handoff only: 1400607f0 reuses an acquired material identity.
    // Loose overlays keep independent reads because their contents can change.
    pub(in crate::app) fn stage_source(&mut self, path: &str, bytes: Vec<u8>, enabled: bool) {
        self.pending_source = enabled.then(|| {
            self.max_source_capacity = self.max_source_capacity.max(bytes.capacity());
            (path.to_owned(), bytes)
        });
    }

    pub(in crate::app) fn take_source(&mut self, path: &str) -> Option<Vec<u8>> {
        let (name, bytes) = self.pending_source.take()?;
        if name != path { return None; }
        if trace_source() {
            self.source_matches += 1;
            self.source_bytes += bytes.len();
            self.source_crc.get_or_insert_with(flate2::Crc::new).update(&bytes);
        }
        if separate_source() { None } else { Some(bytes) }
    }

    pub(in crate::app) fn record_source(&mut self, path: &str, bytes: &[u8], admission: Option<u8>) {
        if trace_source() {
            let crc = match admission { Some(_) => &mut self.admission_crc, None => &mut self.resolved_crc };
            let crc = crc.get_or_insert_with(flate2::Crc::new);
            crc.update(path.as_bytes());
            crc.update(bytes);
            if let Some(value) = admission { crc.update(&[value]); }
        }
    }

    /// End the caster-to-material source handoff before retaining this cache.
    /// All future requests use the typed entries or acquire their own source.
    pub(in crate::app) fn retire_staged_source(&mut self) {
        let retain = std::env::var("DH2_DEBUG_RETAIN_MATERIAL_STAGING").as_deref() == Ok("1");
        let before = self.pending_source.as_ref().map_or(0, |(name, bytes)| name.capacity() + bytes.capacity());
        if !retain { self.pending_source = None; }
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            let after = self.pending_source.as_ref().map_or(0, |(name, bytes)| name.capacity() + bytes.capacity());
            eprintln!("MATERIAL_SOURCE_RETIRE retain={retain} before_bytes={before} after_bytes={after}");
        }
    }

    pub(in crate::app) fn report_source_handoff(&self) {
        if trace_source() {
            eprintln!("MEMORY material source handoff: matches={} bytes={} max_capacity={} signature={:08x} admission={:08x} resolved={:08x}",
                self.source_matches, self.source_bytes, self.max_source_capacity,
                self.source_crc.as_ref().map_or(0, flate2::Crc::sum),
                self.admission_crc.as_ref().map_or(0, flate2::Crc::sum),
                self.resolved_crc.as_ref().map_or(0, flate2::Crc::sum));
        }
    }
    pub(in crate::app) fn raise_image_priority(&mut self, slot: usize, priority: u8) {
        if self.image_priorities.len() <= slot { self.image_priorities.resize(slot + 1, 1); }
        crate::texture::raise_image_priority(&mut self.image_priorities[slot], priority);
    }

    pub(in crate::app) fn image_priority(&self, slot: usize) -> u8 {
        self.image_priorities.get(slot).copied().unwrap_or(1)
    }

    pub(in crate::app) fn get_for(
        &self,
        path: &str,
        geometry: MaterialGeometry,
    ) -> Option<&Option<CachedMaterial>> {
        self.trace_dependency(path, geometry);
        let entry = self.entries.get(path)?;
        let index = geometry.index();
        entry[index].as_ref().or_else(|| {
            let result = entry[1 - index].as_ref().filter(|cached| {
                cached.as_ref().is_none_or(|cached| cached.compatible_geometry_routes)
            });
            if result.is_some() {
                self.trace_dependency(path, if index == 0 { MaterialGeometry::Model } else { MaterialGeometry::World });
            }
            result
        })
    }

    #[cfg(test)]
    pub(in crate::app) fn get(&self, path: &str) -> Option<&Option<CachedMaterial>> {
        self.get_for(path, MaterialGeometry::Model)
    }

    pub(in crate::app) fn insert(
        &mut self,
        path: String,
        geometry: MaterialGeometry,
        material: Option<CachedMaterial>,
    ) {
        self.trace_dependency(&path, geometry);
        self.entries.entry(path).or_default()[geometry.index()] = Some(material);
    }

    /// Runtime workers trace only material identities actually touched by the
    /// current request. Startup and ordinary frame paths retain no trace state.
    pub(in crate::app) fn enable_dependency_trace(&mut self) {
        self.dependency_trace = Some(std::sync::Mutex::new(super::HashSet::new()));
    }
    fn trace_dependency(&self, path: &str, geometry: MaterialGeometry) {
        if let Some(trace) = &self.dependency_trace {
            trace.lock().unwrap_or_else(|error| error.into_inner()).insert((path.to_owned(), geometry));
        }
    }
    pub(in crate::app) fn take_dependencies(&self) -> super::HashSet<(String, MaterialGeometry)> {
        self.dependency_trace.as_ref().map(|trace| std::mem::take(&mut *trace.lock()
            .unwrap_or_else(|error| error.into_inner()))).unwrap_or_default()
    }
    pub(in crate::app) fn disable_dependency_trace(&mut self) { self.dependency_trace = None; }

    /// Remap worker-local identities without publishing them to the live cache.
    pub(in crate::app) fn map_dynamic(&self, other: MaterialCache, slots: &[usize])
        -> Result<Vec<(String, MaterialGeometry, CachedMaterial)>, String> {
        let mut mapped = Vec::new();
        for (name, entries) in other.entries {
            for (index, entry) in entries.into_iter().enumerate() {
                let Some(Some(mut material)) = entry else { continue };
                let geometry = if index == 0 { MaterialGeometry::World } else { MaterialGeometry::Model };
                if self.get_for(&name, geometry).is_some() { continue; }
                for (slot, _) in &mut material.images { *slot = *slots.get(*slot).ok_or("material image mapping is incomplete")?; }
                material.diffuse_image_slot = material.diffuse_image_slot.map(|slot| slots.get(slot).copied()
                    .ok_or("material diffuse image mapping is incomplete")).transpose()?;
                mapped.push((name.clone(), geometry, material));
            }
        }
        Ok(mapped)
    }
    pub(in crate::app) fn remove_dynamic(&mut self, name: &str, geometry: MaterialGeometry) {
        if let Some(entries) = self.entries.get_mut(name) {
            entries[geometry.index()] = None;
            if entries.iter().all(Option::is_none) { self.entries.remove(name); }
        }
    }

    pub(in crate::app) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Logical cache storage retained when the startup owner is transferred
    /// to runtime effect preparation; this is not process RSS.
    pub(in crate::app) fn report_runtime_retention(&self) {
        if trace_source() {
            let paths: usize = self.entries.keys().map(String::capacity).sum();
            let image_slots: usize = self.entries.values().flat_map(|entry| entry.iter())
                .filter_map(Option::as_ref).filter_map(Option::as_ref)
                .map(|material| material.images.capacity()).sum();
            eprintln!("MEMORY runtime material cache: entries={} usable_slots={} path_capacity_bytes={} image_reference_slots={image_slots}",
                self.entries.len(), self.entries.capacity(), paths);
        }
    }

    pub(in crate::app) fn values(&self) -> impl Iterator<Item = &Option<CachedMaterial>> {
        self.entries
            .values()
            .flat_map(|entry| entry.iter().filter_map(Option::as_ref))
    }
    pub(in crate::app) fn named_values(&self) -> impl Iterator<Item = (&str, &CachedMaterial)> {
        self.entries.iter().flat_map(|(name, entries)| entries.iter()
            .filter_map(Option::as_ref).filter_map(Option::as_ref)
            .map(move |material| (name.as_str(), material)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::MaterialHandle;

    fn material(shared: bool, handle: MaterialHandle) -> CachedMaterial {
        CachedMaterial {
            physics_material: None,
            physics_material_source_present: true,
            physics_material_preprocessing_required: false,
            handle,
            diffuse_image_slot: None,
            images: Vec::new(),
            vertex_lighting: false,
            stage_sort: Ok([80f32.to_bits(), 0.5f32.to_bits()]),
            compatible_geometry_routes: shared,
        }
    }

    #[test]
    fn adjusted_sort_does_not_alias_world_colour_and_model_boat_routes() {
        let mut cache = MaterialCache::default();
        let path = "adjusted.material";
        cache.insert(
            path.into(),
            MaterialGeometry::Model,
            Some(material(false, MaterialHandle::BoatVolume(path.into()))),
        );
        assert!(cache.get_for(path, MaterialGeometry::World).is_none());
        let colour = MaterialHandle::Probe(Default::default());
        cache.insert(
            path.into(),
            MaterialGeometry::World,
            Some(material(false, colour.clone())),
        );
        assert!(
            cache
                .get_for(path, MaterialGeometry::World)
                .unwrap()
                .as_ref()
                .unwrap()
                .handle
                == colour
        );
        assert!(matches!(
            cache
                .get_for(path, MaterialGeometry::Model)
                .unwrap()
                .as_ref()
                .unwrap()
                .handle,
            MaterialHandle::BoatVolume(_)
        ));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.values().flatten().count(), 2);
        cache.insert(
            "shared.material".into(),
            MaterialGeometry::World,
            Some(material(true, colour.clone())),
        );
        assert!(
            cache
                .get_for("shared.material", MaterialGeometry::Model)
                .unwrap()
                .as_ref()
                .unwrap()
                .handle
                == colour
        );
        assert_eq!(cache.values().flatten().count(), 3);
    }
}
