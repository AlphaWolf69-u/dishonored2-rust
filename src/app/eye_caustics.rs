//! arkEyeball's `eye/causticMap`: a volume image sampled by BRDF_eye when
//! `eye/internalCaustic` is set. Materials record its source at creation;
//! this owner decodes each source once and binds it to every new material.
use super::ProbeMaterial;
use bevy::{
    asset::RenderAssetUsages,
    platform::collections::HashMap,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use std::sync::Arc;

#[derive(Resource, Default)]
pub(super) struct EyeCausticVolumes(HashMap<Arc<str>, Handle<Image>>);

fn volume(
    catalog: &crate::archive::Catalog,
    source: &str,
    images: &mut Assets<Image>,
) -> Result<Handle<Image>, String> {
    let volume = crate::material::load_3d_image(catalog, source)?;
    let format = match volume.info.format {
        crate::texture::ImageFormat::Bgra8 => TextureFormat::Bgra8Unorm,
        crate::texture::ImageFormat::Rgba8 => TextureFormat::Rgba8Unorm,
        crate::texture::ImageFormat::Rgba8Srgb => TextureFormat::Rgba8UnormSrgb,
        other => return Err(format!("eye caustic volume format {other:?} is not connected: {source}")),
    };
    // One level: the installed caustic volume carries no further mips.
    if volume.info.mip_count != 1 {
        return Err(format!("eye caustic volume has {} levels: {source}", volume.info.mip_count));
    }
    Ok(images.add(Image::new(
        Extent3d {
            width: volume.info.width,
            height: volume.info.height,
            depth_or_array_layers: volume.depth,
        },
        TextureDimension::D3,
        volume.payload,
        format,
        RenderAssetUsages::RENDER_WORLD,
    )))
}

pub(super) fn bind(
    mut events: MessageReader<AssetEvent<ProbeMaterial>>,
    mut materials: ResMut<Assets<ProbeMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut volumes: ResMut<EyeCausticVolumes>,
    scene: Option<Res<super::SceneData>>,
    mut reported: Local<Option<String>>,
) {
    let added: Vec<_> = events
        .read()
        .filter_map(|event| match event {
            AssetEvent::Added { id } => Some(*id),
            _ => None,
        })
        .collect();
    for id in added {
        let Some(source) = materials
            .get(id)
            .filter(|material| material.extension.eye_caustic.is_none())
            .and_then(|material| material.extension.eye_caustic_source.clone())
        else {
            continue;
        };
        let handle = if let Some(handle) = volumes.0.get(&source) {
            handle.clone()
        } else {
            let Some(catalog) = scene.as_ref().and_then(|scene| scene.catalog.clone()) else {
                continue;
            };
            match volume(&catalog, &source, &mut images) {
                Ok(handle) => {
                    volumes.0.insert(source.clone(), handle.clone());
                    handle
                }
                Err(error) => {
                    if reported.as_ref() != Some(&error) {
                        warn!("Eye caustic unavailable: {error}");
                        *reported = Some(error);
                    }
                    continue;
                }
            }
        };
        if let Some(mut material) = materials.get_mut(id) {
            material.extension.eye_caustic = Some(handle);
        }
    }
}
