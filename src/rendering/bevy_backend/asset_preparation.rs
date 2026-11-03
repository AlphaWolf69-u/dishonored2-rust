//! bevy_backend: prepare payloads off-thread using the existing asset owner's
//! handle allocator. Publication transfers each payload once; no second owner,
//! GPU upload path, or independently allocated asset identity is introduced.
use bevy::{
    asset::{Asset, AssetHandleProvider, AssetMut},
    prelude::*,
};
use std::collections::HashMap;

pub trait AssetWrite<A: Asset> {
    fn add(&mut self, value: A) -> Handle<A>;
    fn get(&self, handle: &Handle<A>) -> Option<&A>;
    fn get_mut(&mut self, handle: &Handle<A>) -> Option<&mut A>;
}
impl<A: Asset> AssetWrite<A> for Assets<A> {
    fn add(&mut self, value: A) -> Handle<A> {
        Assets::add(self, value)
    }
    fn get(&self, handle: &Handle<A>) -> Option<&A> {
        Assets::get(self, handle)
    }
    fn get_mut(&mut self, handle: &Handle<A>) -> Option<&mut A> {
        Assets::get_mut(self, handle).map(AssetMut::into_inner)
    }
}
impl<A: Asset> AssetWrite<A> for ResMut<'_, Assets<A>> {
    fn add(&mut self, value: A) -> Handle<A> {
        Assets::add(self, value)
    }
    fn get(&self, handle: &Handle<A>) -> Option<&A> {
        Assets::get(self, handle)
    }
    fn get_mut(&mut self, handle: &Handle<A>) -> Option<&mut A> {
        Assets::get_mut(self, handle).map(AssetMut::into_inner)
    }
}
pub(crate) struct AssetBatch<A: Asset> {
    provider: AssetHandleProvider,
    values: HashMap<AssetId<A>, (Handle<A>, A)>,
}
impl<A: Asset> AssetBatch<A> {
    pub fn new(assets: &Assets<A>) -> Self {
        Self {
            provider: assets.get_handle_provider(),
            values: HashMap::new(),
        }
    }
    pub fn ids(&self, included: impl Fn(&A) -> bool) -> Vec<AssetId<A>> {
        self.values
            .iter()
            .filter_map(|(id, (_, value))| included(value).then_some(*id))
            .collect()
    }
    pub fn publish(self, assets: &mut Assets<A>) {
        for (id, (_handle, value)) in self.values {
            assets.insert(id, value).expect("reserved asset identity");
        }
    }
}
impl<A: Asset> AssetWrite<A> for AssetBatch<A> {
    fn add(&mut self, value: A) -> Handle<A> {
        let handle = self.provider.reserve_handle().typed::<A>();
        self.values.insert(handle.id(), (handle.clone(), value));
        handle
    }
    fn get(&self, handle: &Handle<A>) -> Option<&A> {
        self.values.get(&handle.id()).map(|(_, value)| value)
    }
    fn get_mut(&mut self, handle: &Handle<A>) -> Option<&mut A> {
        self.values.get_mut(&handle.id()).map(|(_, value)| value)
    }
}
