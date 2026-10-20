//! Upload the installed reflection cubemaps without expanding their BC6H data.
use super::{archive, material, probe_lighting, texture, ShaderBuffer};
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{
        Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
    },
};

pub(super) fn load(
    catalog: Option<&archive::Catalog>,
    probes: Option<&probe_lighting::ProbeLightingSet>,
    images: &mut Assets<Image>,
    buffers: &mut Assets<ShaderBuffer>,
) -> Result<(Handle<Image>, Handle<ShaderBuffer>, Option<Handle<Image>>), String> {
    let count = probes.map_or(0, |p| p.reflection_probes.len());
    let mut words = vec![count as u32, 0, 0, 0];
    let mut data = Vec::new();
    let mut spec = None;
    if let Some(probes) = probes {
        for probe in &probes.reflection_probes {
            let catalog = catalog.ok_or("reflection catalog missing")?;
            let path = material::compiled_image_stem(catalog, &probe.cubemap_name)
                .ok_or_else(|| format!("reflection image unavailable: {}", probe.cubemap_name))?;
            let bytes = catalog
                .read(&path)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("reflection cube missing: {path}"))?;
            let cube =
                texture::compressed_inline_cube(&bytes).map_err(|e| format!("{path}: {e}"))?;
            if cube.info.format != texture::ImageFormat::Bc6Unsigned {
                return Err(format!("{path}: expected BC6H unsigned reflection cube"));
            }
            if let Some(previous) = spec {
                if previous != cube.info {
                    return Err(format!("reflection cube descriptors disagree: {path}"));
                }
            }
            spec = Some(cube.info);
            // Bevy's default upload order is layer-major. The archive is mip-major.
            for face in 0..6 {
                let mut offset = 0;
                for level in 0..cube.info.mip_count {
                    let width = (cube.info.width >> level).max(1);
                    let size = cube.info.format.mip_bytes(width, width);
                    data.extend_from_slice(
                        &cube.payload[offset + face * size..offset + (face + 1) * size],
                    );
                    offset += 6 * size;
                }
            }
            words.extend(crate::rendering::environment_reflection::probe_words(probe));
        }
    }
    let (width, mips) = spec.map_or((4, 1), |s| (s.width, s.mip_count));
    if count == 0 {
        data.resize(6 * 16, 0);
        words.resize(20, 0);
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width,
            height: width,
            depth_or_array_layers: count.max(1) as u32 * 6,
        },
        TextureDimension::D2,
        TextureFormat::Bc6hRgbUfloat,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = mips;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::CubeArray),
        ..default()
    });
    image.data = Some(data);
    let mut sampler = ImageSamplerDescriptor::linear();
    sampler.set_address_mode(ImageAddressMode::ClampToEdge);
    sampler.lod_max_clamp = mips.saturating_sub(1) as f32;
    image.sampler = ImageSampler::Descriptor(sampler);
    let lookup = if let Some(catalog) = catalog {
        // The engine binds this resource to texPreIntegratedBRDF_Ward. The
        // vegetation BRDF retains the lookup; ordinary Ward uses its polynomial.
        let path = "generated/image/rendering/brdf/preintegratedbrdf_ward.bimage7";
        let bytes = catalog
            .read(path)
            .map_err(|e| e.to_string())?
            .ok_or("Ward BRDF lookup missing")?;
        let chain = texture::compressed_inline_chain(&bytes)?;
        if chain.info.format != texture::ImageFormat::Rg32Float {
            return Err("Ward BRDF lookup is not RG32Float".into());
        }
        let mut image = Image::new_uninit(
            Extent3d {
                width: chain.info.width,
                height: chain.info.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            TextureFormat::Rg32Float,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.texture_descriptor.mip_level_count = chain.info.mip_count;
        image.data = Some(chain.payload);
        let mut sampler = ImageSamplerDescriptor::linear();
        sampler.set_address_mode(ImageAddressMode::ClampToEdge);
        sampler.lod_max_clamp = 0.;
        image.sampler = ImageSampler::Descriptor(sampler);
        Some(images.add(image))
    } else {
        None
    };
    Ok((
        images.add(image),
        buffers.add(ShaderBuffer::from(words)),
        lookup,
    ))
}
