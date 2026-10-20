//! Append-only pipeline storage: hash descriptions at creation, index draws.
use super::{CachedPipeline, HashMap, PipelineKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct PipelineId(usize);

#[derive(Default)]
pub(super) struct ShadowPipelineStore {
    indices: HashMap<PipelineKey, PipelineId>,
    entries: Vec<CachedPipeline>,
}

impl ShadowPipelineStore {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn alpha_material_count(&self) -> usize {
        self.entries
            .iter()
            .map(|entry| entry.alpha_materials.len())
            .sum()
    }
    pub fn id(&self, key: &PipelineKey) -> Option<PipelineId> {
        self.indices.get(key).copied()
    }
    pub fn by_id(&self, id: PipelineId) -> &CachedPipeline {
        &self.entries[id.0]
    }
    pub fn by_id_mut(&mut self, id: PipelineId) -> &mut CachedPipeline {
        &mut self.entries[id.0]
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut CachedPipeline> {
        self.entries.iter_mut()
    }
    pub fn insert(&mut self, key: PipelineKey, value: CachedPipeline) -> PipelineId {
        assert!(
            !self.indices.contains_key(&key),
            "pipeline IDs cannot be replaced"
        );
        let id = PipelineId(self.entries.len());
        self.entries.push(value);
        self.indices.insert(key, id);
        id
    }
    #[cfg(test)]
    pub fn get_mut(&mut self, key: &PipelineKey) -> Option<&mut CachedPipeline> {
        self.id(key).map(|id| self.by_id_mut(id))
    }
}
