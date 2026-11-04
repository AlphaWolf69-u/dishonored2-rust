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
                .ok_or_else(|| format!("missing resident image {path}"))?;
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

/// Registry ownership is separate from each movie's binding reference
/// (14005feb0 vs 14101d160). Init/common images survive map purge; movie/frame
/// leases keep still-used map images alive until the last consumer retires.
#[derive(Resource, Default)]
pub(super) struct UiImages {
    pub resident: HashMap<String, Handle<Image>>,
    pub aliases: HashMap<String, Handle<Image>>,
    bindings: HashMap<String, std::sync::Weak<crate::flash::iggy_functions::TextureBinding>>,
}
impl UiImages {
    fn acquire(&mut self, catalog: &archive::Catalog, name: &str,
        images: &mut Assets<Image>) -> Result<crate::flash::iggy_functions::BoundTexture, String> {
        let path = crate::material::image_load_resource(catalog, name)
            .ok_or_else(|| format!("unresolved resident image source {name}"))?;
        if let Some(binding) = self.bindings.get(&path).and_then(std::sync::Weak::upgrade) {
            // A prepared/outgoing movie can outlive a registry purge. Re-admit
            // its live identity rather than uploading the same image again.
            let handle = binding.owner::<Handle<Image>>().ok_or("UI image owner unavailable")?;
            self.resident.insert(path, handle.clone());
            self.aliases.insert(name.to_owned(), handle.clone());
            return Ok(binding);
        }
        let handle = prepare_image(catalog, name, images, &mut self.resident, &mut self.aliases)?;
        let image = images.get(&handle).ok_or("UI image descriptor unavailable")?;
        let binding = std::sync::Arc::new(crate::flash::iggy_functions::TextureBinding::new(
            [image.width() as f64, image.height() as f64], handle));
        self.bindings.insert(path, std::sync::Arc::downgrade(&binding));
        Ok(binding)
    }

    pub(super) fn release_level(&mut self, shared: &super::texture_streaming::SharedImages) {
        // Host counterpart of map-class purge (140060180). Live Iggy bindings
        // and submissions retain independent leases through this purge.
        let before = self.resident.len();
        self.resident.retain(|path, _| shared.contains(path));
        let ids: std::collections::HashSet<_> = self.resident.values().map(Handle::id).collect();
        self.aliases.retain(|_, handle| ids.contains(&handle.id()));
        self.bindings.retain(|_, binding| binding.strong_count() != 0);
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY UI image registry purge: released={} retained_common={} live_bindings={}",
                before - self.resident.len(), self.resident.len(), self.bindings.len());
        }
    }
}

pub(super) fn publish_ui_images(
    mut frame: ResMut<crate::rendering::bevy_backend::flash_ui::FlashUiFrame>,
    mut exit: MessageWriter<AppExit>,
) {
    if !frame.is_changed() { return; }
    let Some(frame) = frame.0.as_mut() else { return; };
    let mut named = HashMap::with_capacity(frame.list.named_images.len());
    for (name, binding) in &frame.list.named_images {
        let Some(image) = binding.owner::<Handle<Image>>() else {
            error!("UI image {name}: renderer owner unavailable");
            exit.write(AppExit::error());
            return;
        };
        named.insert(name.clone(), image.clone());
    }
    frame.game_images = std::sync::Arc::new(named);
}

/// Synchronous Iggy callback, fulfilled on the main world that owns Assets.
/// The movie worker can wait without blocking the window/event loop.
pub(super) struct UiTextureRequest {
    pub name: String,
    pub reply: std::sync::mpsc::SyncSender<Result<crate::flash::iggy_functions::BoundTexture, String>>,
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
            let result = self.cache.acquire(catalog, &request.name, &mut self.images);
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
