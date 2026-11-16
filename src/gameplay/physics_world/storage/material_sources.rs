//! 853b80 resolves a hit's actual component (+10) material table. Geometry
//! aliases may be shared by components with different tables; body generations
//! and userData identify the owner, never the shape resource index.
use crate::gameplay::{
    collision_hit::ShapeHitInfo, materials::MaterialOwner, shape_tags::ShapeTagSource,
};
use std::{collections::HashMap, sync::Arc};

struct Source {
    user_data: u64,
    materials: Arc<[u16]>,
}
#[derive(Default)]
pub(crate) struct MaterialSources {
    bodies: HashMap<u32, Source>,
}
impl MaterialSources {
    pub(crate) fn register(
        &mut self,
        body: u32,
        user_data: u64,
        materials: Arc<[u16]>,
    ) -> Result<(), String> {
        if let Some(source) = self.bodies.get(&body) {
            if source.user_data == user_data && Arc::ptr_eq(&source.materials, &materials) {
                return Ok(());
            }
            return Err(
                "collision body material source was already registered by another owner".into(),
            );
        }
        self.bodies.insert(
            body,
            Source {
                user_data,
                materials,
            },
        );
        Ok(())
    }
    pub(crate) fn remove(&mut self, body: u32) {
        self.bodies.remove(&body);
    }
    fn mapped(&self, hit: &ShapeHitInfo, index: usize) -> Result<u16, String> {
        let body = hit
            .body_id
            .ok_or("mapped shape tag has no collision body")?;
        let source = self
            .bodies
            .get(&body)
            .ok_or("collision body has no component material table")?;
        if source.user_data != hit.user_data {
            return Err("collision material table userData does not match body".into());
        }
        source
            .materials
            .get(index)
            .copied()
            .ok_or_else(|| format!("component material index {index} out of range"))
    }
}

/// Borrow the canonical table and declaration library for a query or collision
/// pass. No material IDs or resource references are copied per hit.
pub(crate) struct MaterialSourceView<'a> {
    pub sources: &'a MaterialSources,
    pub materials: &'a MaterialOwner<usize, usize>,
}
impl ShapeTagSource for MaterialSourceView<'_> {
    fn mapped_material(&self, _index: usize) -> Result<u16, String> {
        Err("component material mapping requires the actual hit body".into())
    }
    fn mapped_material_for_hit(&self, hit: &ShapeHitInfo, index: usize) -> Result<u16, String> {
        self.sources.mapped(hit, index)
    }
    fn material_flags(&self, material: u16) -> Result<u16, String> {
        self.materials.shape_filter_flags(material)
    }
}

impl super::WorldStorage {
    pub(crate) fn register_body_materials(
        &mut self,
        body: u32,
        materials: Arc<[u16]>,
    ) -> Result<(), String> {
        let state = self
            .bodies
            .get(body)
            .ok_or("material source body is not live")?;
        self.material_sources
            .register(body, state.user_data, materials)
    }
}
