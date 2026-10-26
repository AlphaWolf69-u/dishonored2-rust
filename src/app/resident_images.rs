//! Full-mip resident images resolved through the installed archive image loader.
use super::{
    archive,
    texture_streaming::{TextureReadTarget, read_streamed_diffuse_at_target},
};
use bevy::prelude::*;
use std::collections::HashMap;

pub(super) fn prepare_image(
    catalog: &archive::Catalog,
    name: &str,
    images: &mut Assets<Image>,
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
