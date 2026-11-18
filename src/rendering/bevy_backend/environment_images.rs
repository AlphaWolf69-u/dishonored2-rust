//! Admission receipts for environment projections prepared by canonical image
//! owners. This publication owns no decoder, image handle or source lease.
use bevy::{prelude::*,render::{extract_resource::{ExtractResource,ExtractResourcePlugin},
    render_asset::RenderAssets,texture::GpuImage,Render,RenderApp,RenderSystems}};
use std::sync::{Arc,Weak,atomic::{AtomicBool,AtomicU64,Ordering}};
use crate::rendering::environment_resources::PreparedEnvironment;
pub struct Projection {pub environment:Weak<PreparedEnvironment>,pub images:Vec<AssetId<Image>>,pub lifetime:Weak<AtomicBool>}
#[derive(Resource,Clone,Default,ExtractResource)]
pub struct EnvironmentImages {
    pub generation:u64,pub projections:Arc<Vec<Projection>>,pub completed:Arc<AtomicU64>,
    /// Host receipt observation, separate from all-new-images-ready completion.
    pub observed:Arc<AtomicU64>,
}
impl EnvironmentImages {
    pub fn withdrawal_observed(&self,required:u64)->bool {
        let observed=self.observed.load(Ordering::Acquire);
        // Host monotonic receipt sequence reserves zero. Accept a later real
        // snapshot too, including counter wrap, without waiting for its uploads.
        observed!=0 && observed.wrapping_sub(required)<(1u64<<63)
    }
}
pub fn register(app:&mut App){
    app.init_resource::<EnvironmentImages>().add_plugins(ExtractResourcePlugin::<EnvironmentImages>::default());
    if let Some(render)=app.get_sub_app_mut(RenderApp){render.add_systems(Render,acknowledge.in_set(RenderSystems::PrepareResources));}
}
fn acknowledge(inputs:Res<EnvironmentImages>,images:Res<RenderAssets<GpuImage>>){
    // ExtractSchedule has replaced changed receipt/display resources before
    // PrepareResources. Display/Lens Handle clones and actual GPU bind groups
    // retain any still-referenced images; this is withdrawal, not GPU idle.
    if inputs.observed.load(Ordering::Relaxed)!=inputs.generation {
        inputs.observed.store(inputs.generation,Ordering::Release);
    }
    if inputs.completed.load(Ordering::Acquire)==inputs.generation{return;}
    let mut ready=true;
    for projection in inputs.projections.iter(){
        let Some(live)=projection.lifetime.upgrade().filter(|live|live.load(Ordering::Acquire))else{continue};
        let Some(environment)=projection.environment.upgrade()else{continue};
        if environment.prepared.load(Ordering::Acquire){continue;}
        if projection.images.iter().all(|image|images.get(*image).is_some()){
            // Typed parameters/projection handles were published before this
            // receipt. Every demanded image now has its actual GPU view.
            environment.prepared.store(true,Ordering::Release);
            // A concurrently retiring main-world projection cannot receive a
            // stale ready acknowledgement from an older extracted snapshot.
            if !live.load(Ordering::Acquire){environment.prepared.store(false,Ordering::Release);}
        }else{ready=false;}
    }
    if ready{inputs.completed.store(inputs.generation,Ordering::Release);}
}
