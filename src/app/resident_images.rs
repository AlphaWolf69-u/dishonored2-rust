//! Full-mip resident images resolved through the installed archive image loader.
use crate::rendering::bevy_backend::asset_preparation::AssetWrite;
use super::{
    archive,
    texture_streaming::{TextureReadTarget, read_streamed_diffuse_at_target},
};
use bevy::prelude::*;
use std::collections::HashMap;

pub(super) fn prepare_image(
    catalog: &archive::Catalog,
    name: &str,
    images: &mut dyn AssetWrite<Image>,
    resident: &mut HashMap<String, Handle<Image>>,
    aliases: &mut HashMap<String, Handle<Image>>,
) -> Result<Handle<Image>, String> {
    if let Some(handle) = aliases.get(name) {
        return Ok(handle.clone());
    }
    // Preserve authored image option suffixes and accept compiled BIM paths.
    let path = crate::material::image_load_resource(catalog, name)
        .ok_or_else(|| format!("unresolved resident image source {name}"))?;
    if path == "_default" && !name.eq_ignore_ascii_case("_default") {
        warn!("Resident image {name}: resource missing; using the image loader's default texture");
    }
    let handle = if let Some(handle) = resident.get(&path) {
        handle.clone()
    } else {
        let mut loaded =
            read_streamed_diffuse_at_target(catalog, &path, TextureReadTarget::Initial(Some(0)))?
                .ok_or_else(|| format!("missing resident resident image {path}"))?;
        loaded.image.data = Some(
            loaded
                .uploads
                .into_iter()
                .flat_map(|mip| mip.payload)
                .collect(),
        );
        let handle = images.add(loaded.image);
        resident.insert(path, handle.clone());
        handle
    };
    aliases.insert(name.to_owned(), handle.clone());
    aliases.insert(
        name.trim_end_matches(".curvefloat").to_owned(),
        handle.clone(),
    );
    Ok(handle)
}

/// UI consumers request the same installed-image owner as scene consumers.
/// Retain the handles across bootstrap/loading/pause; no UI-owned copy/upload.
#[derive(Resource, Default)]
pub(super) struct UiImages {
    pub resident: HashMap<String, Handle<Image>>,
    pub aliases: HashMap<String, Handle<Image>>,
    dimensions: HashMap<bevy::asset::AssetId<Image>, [f64; 2]>,
}

pub(super) fn publish_ui_images(
    mut frame: ResMut<crate::rendering::bevy_backend::flash_ui::FlashUiFrame>,
    mut cache: ResMut<UiImages>,
    mut images: ResMut<Assets<Image>>,
    loading: Res<super::loading_screen::Acquisition>,
    mut exit: MessageWriter<AppExit>,
) {
    if !frame.is_changed() {
        return;
    }
    let Some(frame) = frame.0.as_mut() else {
        return;
    };
    let mut named = HashMap::new();
    for batch in &frame.list.batches {
        let crate::flash::render::Texture::Game(name) = &batch.texture else {
            continue;
        };
        if named.contains_key(name) {
            continue;
        }
        let UiImages {
            resident, aliases, ..
        } = &mut *cache;
        match prepare_image(&loading.catalog, name, &mut images, resident, aliases) {
            Ok(image) => {
                named.insert(name.clone(), image);
            }
            Err(error) => {
                error!("UI image acquisition failed: {error}");
                exit.write(AppExit::error());
                return;
            }
        }
    }
    frame.game_images = std::sync::Arc::new(named);
}

/// Synchronous Iggy callback, fulfilled on the main world that owns Assets.
/// The movie worker can wait without blocking the window/event loop.
pub(super) struct UiTextureRequest {
    pub name: String,
    pub reply: std::sync::mpsc::SyncSender<Result<[f64; 2], String>>,
}

#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct UiImageService<'w> {
    images: ResMut<'w, Assets<Image>>,
    cache: ResMut<'w, UiImages>,
}
impl UiImageService<'_> {
    pub(super) fn service(
        &mut self,
        pause: &super::pause_menu::PauseMenu,
        catalog: &archive::Catalog,
    ) {
        for request in pause.texture_requests() {
            let UiImages {
                resident, aliases, ..
            } = &mut *self.cache;
            let result = prepare_image(catalog, &request.name, &mut self.images, resident, aliases)
                .and_then(|handle| {
                    if let Some(extent) = self.cache.dimensions.get(&handle.id()) {
                        return Ok(*extent);
                    }
                    let image = self
                        .images
                        .get(&handle)
                        .ok_or("UI image descriptor unavailable")?;
                    let extent = [image.width() as f64, image.height() as f64];
                    self.cache.dimensions.insert(handle.id(), extent);
                    Ok(extent)
                });
            if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                eprintln!("LOAD UI texture binding: {} {result:?}", request.name);
            }
            let _ = request.reply.send(result);
        }
    }
}

pub(super) fn service_ui_images(
    mut service: UiImageService,
    pause: NonSend<super::pause_menu::PauseMenu>,
    state: Res<super::loading_screen::Acquisition>,
) {
    service.service(&pause, &state.catalog);
}
