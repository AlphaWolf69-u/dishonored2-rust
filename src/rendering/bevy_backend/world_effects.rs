//! World FX resources shared with the renderer. Graph playback owns activation;
//! prepared level declarations alone never create visible particle instances.
use crate::rendering::particles::world_sources::WorldFxSources;
use bevy::{prelude::Resource, render::extract_resource::ExtractResource};
use std::sync::Arc;

#[derive(Resource)]
pub struct WorldEffectGraphs(pub Vec<crate::assets::entities::effects::FxGraph>);

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct WorldEffectSources(pub Arc<WorldFxSources>);
