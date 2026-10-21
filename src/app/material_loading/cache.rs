//! Share GPU materials when their world/model routes agree; keep distinct
//! variants when the two source selectors assign different passes.
use super::{CachedMaterial, HashMap};

#[derive(Clone, Copy)]
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
}

impl MaterialCache {
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

    pub(in crate::app) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(in crate::app) fn values(&self) -> impl Iterator<Item = &Option<CachedMaterial>> {
        self.entries
            .values()
            .flat_map(|entry| entry.iter().filter_map(Option::as_ref))
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
