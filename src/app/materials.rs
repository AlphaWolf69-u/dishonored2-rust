//! Materials.
use super::{
    surface_gpu, AsBindGroup, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
    MeshVertexAttribute, MeshVertexBufferLayoutRef, RenderPipelineDescriptor, ShaderBuffer,
    ShaderRef, SpecializedMeshPipelineError, VertexFormat, DECAL_SHADER_HANDLE,
    PROBE_PREPASS_SHADER_HANDLE, PROBE_SHADER_HANDLE,
};
use bevy::prelude::*;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bind_group_data(SurfaceKey)]
pub(super) struct ProbeExtension {
    #[storage(100, read_only)]
    pub(super) sh: Handle<ShaderBuffer>,
    #[storage(119, read_only)]
    pub(super) vertex_probes: Handle<ShaderBuffer>,
    #[storage(120, read_only)]
    pub(super) native_shadow: Handle<ShaderBuffer>,
    #[storage(126, read_only)]
    pub(super) native_local_shadow: Handle<ShaderBuffer>,
    #[texture(127, dimension = "2d_array", sample_type = "depth")]
    #[sampler(128, sampler_type = "comparison")]
    pub(super) native_local_shadow_map: Option<Handle<Image>>,
    #[texture(121, dimension = "2d_array", sample_type = "depth")]
    #[sampler(122, sampler_type = "comparison")]
    pub(super) native_shadow_map: Option<Handle<Image>>,
    #[texture(123)]
    pub(super) native_screen_map: Option<Handle<Image>>,
    #[texture(124)]
    #[sampler(125)]
    pub(super) native_cloud_map: Option<Handle<Image>>,
    #[uniform(101)]
    pub(super) surface: surface_gpu::SurfaceUniform,
    #[texture(102)]
    #[sampler(117)]
    pub(super) diffuse0: Option<Handle<Image>>,
    #[texture(103)]
    pub(super) diffuse1: Option<Handle<Image>>,
    #[texture(104)]
    pub(super) diffuse2: Option<Handle<Image>>,
    #[texture(105)]
    pub(super) bump0: Option<Handle<Image>>,
    #[texture(106)]
    pub(super) bump1: Option<Handle<Image>>,
    #[texture(107)]
    pub(super) bump2: Option<Handle<Image>>,
    #[texture(108)]
    #[sampler(118)]
    pub(super) gloss0: Option<Handle<Image>>,
    #[texture(109)]
    pub(super) gloss1: Option<Handle<Image>>,
    #[texture(110)]
    pub(super) gloss2: Option<Handle<Image>>,
    #[texture(111)]
    pub(super) metallic0: Option<Handle<Image>>,
    #[texture(112)]
    pub(super) metallic1: Option<Handle<Image>>,
    #[texture(113)]
    pub(super) metallic2: Option<Handle<Image>>,
    #[texture(114)]
    pub(super) mask0: Option<Handle<Image>>,
    #[texture(115)]
    pub(super) mask1: Option<Handle<Image>>,
    #[texture(116)]
    pub(super) mask2: Option<Handle<Image>>,
    #[texture(134)]
    pub(super) specular0: Option<Handle<Image>>,
    #[texture(135)]
    pub(super) specular1: Option<Handle<Image>>,
    #[texture(136)]
    pub(super) specular2: Option<Handle<Image>>,
    #[texture(137)]
    pub(super) detail_bump: Option<Handle<Image>>,
    #[texture(138)]
    pub(super) emissive_map: Option<Handle<Image>>,
    #[texture(139)]
    pub(super) emissive_mask: Option<Handle<Image>>,
    #[texture(141)]
    #[sampler(140)]
    pub(super) anisotropic_sampler_image: Option<Handle<Image>>,
}

// Original common vertex shader transforms N/T/B independently with the
// model's linear rows. A separate bitangent avoids replacing that rule with
// inverse-transpose normals or a post-transform cross product under shear.
pub(super) const DH2_BITANGENT: MeshVertexAttribute =
    MeshVertexAttribute::new("DH2_Bitangent", 0x4448_3200, VertexFormat::Float32x3);

use surface_gpu::SurfaceKey;

impl From<&ProbeExtension> for SurfaceKey {
    fn from(extension: &ProbeExtension) -> Self {
        Self::from(&extension.surface)
    }
}

pub(super) fn specialize_surface(
    descriptor: &mut RenderPipelineDescriptor,
    key: surface_gpu::SurfaceKey,
) {
    if let Some(fragment) = descriptor.fragment.as_mut() {
        // Optional numerical light attribution; normal rendering has no branch.
        if let Ok(component) = std::env::var("DH2_TRACE_LIGHT_COMPONENT")
            .unwrap_or_default()
            .parse::<u32>()
        {
            fragment.shader_defs.push(bevy::shader::ShaderDefVal::UInt(
                "DH2_TRACE_LIGHT_COMPONENT".into(),
                component,
            ));
        }
        for (name, value) in surface_gpu::SurfaceKey::NAMES.into_iter().zip(key.0) {
            fragment
                .shader_defs
                .push(bevy::shader::ShaderDefVal::UInt(name.into(), value));
        }
    }
}

pub(super) fn authored_basis_pipeline(
    descriptor: &mut RenderPipelineDescriptor,
    layout: &MeshVertexBufferLayoutRef,
) -> Result<(), SpecializedMeshPipelineError> {
    if layout.0.contains(DH2_BITANGENT) {
        let extra = layout
            .0
            .get_layout(&[DH2_BITANGENT.at_shader_location(8)])?;
        descriptor.vertex.buffers[0]
            .attributes
            .extend(extra.attributes);
        descriptor
            .vertex
            .shader_defs
            .push("DH2_AUTHORED_BASIS".into());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push("DH2_AUTHORED_BASIS".into());
        }
    }
    Ok(())
}

pub(super) fn specialize_native_vertex_lighting(
    descriptor: &mut RenderPipelineDescriptor,
    layout: &MeshVertexBufferLayoutRef,
    key: surface_gpu::SurfaceKey,
) {
    let native_lighting = matches!(key.0[6], 1 | 2)
        && layout.0.contains(Mesh::ATTRIBUTE_NORMAL)
        && layout.0.contains(Mesh::ATTRIBUTE_TANGENT)
        && descriptor.fragment.as_ref().is_some_and(|fragment| {
            fragment.shader == PROBE_SHADER_HANDLE || fragment.shader == DECAL_SHADER_HANDLE
        });
    if native_lighting {
        descriptor
            .vertex
            .shader_defs
            .push("DH2_VERTEX_PROBE".into());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push("DH2_VERTEX_PROBE".into());
        }
        for (name, value) in surface_gpu::SurfaceKey::NAMES.into_iter().zip(key.0) {
            descriptor
                .vertex
                .shader_defs
                .push(bevy::shader::ShaderDefVal::UInt(name.into(), value));
        }
    }
}

impl MaterialExtension for ProbeExtension {
    fn fragment_shader() -> ShaderRef {
        PROBE_SHADER_HANDLE.into()
    }

    fn prepass_fragment_shader() -> ShaderRef {
        PROBE_PREPASS_SHADER_HANDLE.into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push("DH2_NATIVE_LOCAL_SHADOWS".into());
        }
        specialize_native_vertex_lighting(descriptor, layout, key.bind_group_data);
        specialize_surface(descriptor, key.bind_group_data);
        authored_basis_pipeline(descriptor, layout)
    }

    fn enable_prepass() -> bool {
        true
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bind_group_data(DecalKey)]
pub(super) struct DecalExtension {
    pub(super) bias: i32,
    #[storage(100, read_only)]
    pub(super) sh: Handle<ShaderBuffer>,
    #[storage(119, read_only)]
    pub(super) vertex_probes: Handle<ShaderBuffer>,
    #[storage(120, read_only)]
    pub(super) native_shadow: Handle<ShaderBuffer>,
    #[storage(126, read_only)]
    pub(super) native_local_shadow: Handle<ShaderBuffer>,
    #[texture(127, dimension = "2d_array", sample_type = "depth")]
    #[sampler(128, sampler_type = "comparison")]
    pub(super) native_local_shadow_map: Option<Handle<Image>>,
    #[texture(121, dimension = "2d_array", sample_type = "depth")]
    #[sampler(122, sampler_type = "comparison")]
    pub(super) native_shadow_map: Option<Handle<Image>>,
    #[texture(123)]
    pub(super) native_screen_map: Option<Handle<Image>>,
    #[texture(124)]
    #[sampler(125)]
    pub(super) native_cloud_map: Option<Handle<Image>>,
    #[uniform(101)]
    pub(super) surface: surface_gpu::SurfaceUniform,
    #[texture(102)]
    #[sampler(117)]
    pub(super) diffuse0: Option<Handle<Image>>,
    #[texture(103)]
    pub(super) diffuse1: Option<Handle<Image>>,
    #[texture(104)]
    pub(super) diffuse2: Option<Handle<Image>>,
    #[texture(105)]
    pub(super) bump0: Option<Handle<Image>>,
    #[texture(106)]
    pub(super) bump1: Option<Handle<Image>>,
    #[texture(107)]
    pub(super) bump2: Option<Handle<Image>>,
    #[texture(108)]
    #[sampler(118)]
    pub(super) gloss0: Option<Handle<Image>>,
    #[texture(109)]
    pub(super) gloss1: Option<Handle<Image>>,
    #[texture(110)]
    pub(super) gloss2: Option<Handle<Image>>,
    #[texture(111)]
    pub(super) metallic0: Option<Handle<Image>>,
    #[texture(112)]
    pub(super) metallic1: Option<Handle<Image>>,
    #[texture(113)]
    pub(super) metallic2: Option<Handle<Image>>,
    #[texture(114)]
    pub(super) mask0: Option<Handle<Image>>,
    #[texture(115)]
    pub(super) mask1: Option<Handle<Image>>,
    #[texture(116)]
    pub(super) mask2: Option<Handle<Image>>,
    #[texture(134)]
    pub(super) specular0: Option<Handle<Image>>,
    #[texture(135)]
    pub(super) specular1: Option<Handle<Image>>,
    #[texture(136)]
    pub(super) specular2: Option<Handle<Image>>,
    #[texture(137)]
    pub(super) detail_bump: Option<Handle<Image>>,
    #[texture(138)]
    pub(super) emissive_map: Option<Handle<Image>>,
    #[texture(139)]
    pub(super) emissive_mask: Option<Handle<Image>>,
    #[texture(141)]
    #[sampler(140)]
    pub(super) anisotropic_sampler_image: Option<Handle<Image>>,
}

pub(super) trait NativeImageAccess {
    fn image_mut(&mut self, role: surface_gpu::ImageRole) -> &mut Option<Handle<Image>>;
}
macro_rules! native_image_access {
    ($ty:ty) => {
        impl NativeImageAccess for $ty {
            fn image_mut(&mut self, role: surface_gpu::ImageRole) -> &mut Option<Handle<Image>> {
                match role.0 {
                    0 => &mut self.diffuse0,
                    1 => &mut self.diffuse1,
                    2 => &mut self.diffuse2,
                    3 => &mut self.bump0,
                    4 => &mut self.bump1,
                    5 => &mut self.bump2,
                    6 => &mut self.gloss0,
                    7 => &mut self.gloss1,
                    8 => &mut self.gloss2,
                    9 => &mut self.metallic0,
                    10 => &mut self.metallic1,
                    11 => &mut self.metallic2,
                    12 => &mut self.mask0,
                    13 => &mut self.mask1,
                    14 => &mut self.mask2,
                    16 => &mut self.specular0,
                    17 => &mut self.specular1,
                    18 => &mut self.specular2,
                    19 => &mut self.detail_bump,
                    20 => &mut self.emissive_map,
                    21 => &mut self.emissive_mask,
                    _ => panic!("invalid authored image role"),
                }
            }
        }
    };
}
native_image_access!(ProbeExtension);
native_image_access!(DecalExtension);

pub(super) trait NativeShadowImageAccess {
    fn sync_shadow_images(
        &mut self,
        target: &Option<Handle<Image>>,
        screen: &Option<Handle<Image>>,
        cloud: &Option<Handle<Image>>,
        local: &Option<Handle<Image>>,
    );
}
macro_rules! native_shadow_image_access {
    ($ty:ty) => {
        impl NativeShadowImageAccess for $ty {
            fn sync_shadow_images(
                &mut self,
                target: &Option<Handle<Image>>,
                screen: &Option<Handle<Image>>,
                cloud: &Option<Handle<Image>>,
                local: &Option<Handle<Image>>,
            ) {
                if &self.native_shadow_map != target {
                    self.native_shadow_map = target.clone();
                }
                if &self.native_screen_map != screen {
                    self.native_screen_map = screen.clone();
                }
                if &self.native_cloud_map != cloud {
                    self.native_cloud_map = cloud.clone();
                }
                if &self.native_local_shadow_map != local {
                    self.native_local_shadow_map = local.clone();
                }
            }
        }
    };
}
native_shadow_image_access!(ProbeExtension);
native_shadow_image_access!(DecalExtension);

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub(super) struct DecalKey {
    pub(super) bias: i32,
    pub(super) surface: surface_gpu::SurfaceKey,
}

impl From<&DecalExtension> for DecalKey {
    fn from(extension: &DecalExtension) -> Self {
        Self {
            bias: extension.bias,
            surface: surface_gpu::SurfaceKey::from(&extension.surface),
        }
    }
}

impl MaterialExtension for DecalExtension {
    fn fragment_shader() -> ShaderRef {
        DECAL_SHADER_HANDLE.into()
    }

    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            // Bevy's StandardMaterial.depth_bias only sorts transparent draws;
            // the Void arkDecal z-bias needs an actual GPU depth offset.
            depth.bias.constant = key.bind_group_data.bias;
        }
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push("DH2_NATIVE_LOCAL_SHADOWS".into());
        }
        specialize_native_vertex_lighting(descriptor, layout, key.bind_group_data.surface);
        specialize_surface(descriptor, key.bind_group_data.surface);
        authored_basis_pipeline(descriptor, layout)
    }
}
