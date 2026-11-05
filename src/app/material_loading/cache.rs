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
    entries: HashMap<String, [Option<Option<CachedMaterial>>; 2]>,
    image_priorities: Vec<u8>,
    pending_source: Option<(String, Vec<u8>)>,
    source_matches: usize,
    source_bytes: usize,
    max_source_capacity: usize,
    source_crc: Option<flate2::Crc>,
    admission_crc: Option<flate2::Crc>,
    resolved_crc: Option<flate2::Crc>,
}

impl MaterialCache {
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
        let entry = self.entries.get(path)?;
        let index = geometry.index();
        entry[index].as_ref().or_else(|| {
            entry[1 - index].as_ref().filter(|cached| {
                cached
                    .as_ref()
                    .is_none_or(|cached| cached.compatible_geometry_routes)
            })
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
        self.entries.entry(path).or_default()[geometry.index()] = Some(material);
    }

    /// Transfer worker-local identities to existing scene slots only after the
    /// coordinator has verified that every mapped slot is resident.
    pub(in crate::app) fn merge_dynamic(&mut self, other: MaterialCache, slots: &[usize])
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
        for (name, geometry, material) in &mapped { self.insert(name.clone(), *geometry, Some(material.clone())); }
        Ok(mapped)
    }
    pub(in crate::app) fn keys(&self) -> Vec<(String, MaterialGeometry)> {
        self.entries.iter().flat_map(|(name, entries)| entries.iter().enumerate().filter_map(move |(index, entry)| {
            entry.as_ref().and_then(Option::as_ref).map(|_| (name.clone(),
                if index == 0 { MaterialGeometry::World } else { MaterialGeometry::Model }))
        })).collect()
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
