//! Optional material-owned specialization for depth-only camera rendering.
use bevy_ecs::prelude::Resource;
use bevy_material::key::ErasedMaterialKey;
use bevy_platform::collections::HashMap;
use core::{
    any::TypeId,
    hash::Hash,
    sync::atomic::{AtomicU64, Ordering},
};

type Normalize = Box<dyn Fn(&ErasedMaterialKey) -> ErasedMaterialKey + Send + Sync>;
type MaterialFree = Box<dyn Fn(&ErasedMaterialKey) -> bool + Send + Sync>;

/// Register before rendering begins. A material may remove specialization
/// fields which its depth-only shaders provably do not consume, and declare
/// when its depth shader consumes no material bindings at all. Other passes
/// retain their ordinary keys and material dependencies.
#[derive(Resource, Default)]
pub struct DepthPrepassMaterialKeys {
    normalizers: HashMap<TypeId, Normalize>,
    material_free: HashMap<TypeId, MaterialFree>,
    normalized: AtomicU64,
    omitted: AtomicU64,
}

impl DepthPrepassMaterialKeys {
    pub fn register<M: crate::Material>(&mut self, normalize: fn(M::Data) -> M::Data)
    where
        M::Data: Clone + Hash + PartialEq,
    {
        self.normalizers.insert(
            TypeId::of::<M>(),
            Box::new(move |key| ErasedMaterialKey::new(normalize(key.to_key::<M::Data>()))),
        );
    }

    /// The predicate must guarantee that the depth vertex shader requires no
    /// material resources and that its fragment stage has no effect on depth.
    pub fn register_material_free<M: crate::Material>(&mut self, predicate: fn(&M::Data) -> bool)
    where
        M::Data: Clone,
    {
        self.material_free.insert(
            TypeId::of::<M>(),
            Box::new(move |key| predicate(&key.to_key::<M::Data>())),
        );
    }

    pub fn normalized_keys(&self) -> u64 {
        self.normalized.load(Ordering::Relaxed)
    }
    pub fn omitted_materials(&self) -> u64 {
        self.omitted.load(Ordering::Relaxed)
    }

    pub(super) fn normalize(&self, material: TypeId, key: &ErasedMaterialKey) -> ErasedMaterialKey {
        match self.normalizers.get(&material) {
            Some(normalize) => {
                self.normalized.fetch_add(1, Ordering::Relaxed);
                normalize(key)
            }
            None => key.clone(),
        }
    }

    pub(crate) fn material_is_unused(&self, material: TypeId, key: &ErasedMaterialKey) -> bool {
        self.material_free
            .get(&material)
            .is_some_and(|predicate| predicate(key))
    }

    pub(super) fn record_omission(&self) {
        self.omitted.fetch_add(1, Ordering::Relaxed);
    }
}
