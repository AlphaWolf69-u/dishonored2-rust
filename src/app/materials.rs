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
    #[storage(167, read_only)]
    pub(super) fog_objects: Handle<ShaderBuffer>,
    #[storage(161, read_only)]
    pub(super) fog_parameters: Handle<ShaderBuffer>,
    #[texture(162)]
    pub(super) fog_world: Option<Handle<Image>>,
    #[texture(163)]
    #[sampler(166)]
    pub(super) fog_canvas: Option<Handle<Image>>,
    #[texture(164)]
    pub(super) fog_curve: Option<Handle<Image>>,
    #[texture(165, sample_type = "depth")]
    pub(super) fog_depth: Option<Handle<Image>>,
    #[texture(159)]
    #[sampler(160)]
    pub(super) reflection_brdf_lookup: Option<Handle<Image>>,
    #[texture(156, dimension = "cube_array")]
    #[sampler(157)]
    pub(super) reflections: Option<Handle<Image>>,
    #[storage(158, read_only)]
    pub(super) reflection_planes: Handle<ShaderBuffer>,
    #[storage(154, read_only)]
    pub(super) material_exposure: Handle<ShaderBuffer>,
    #[storage(100, read_only)]
    pub(super) sh: Handle<ShaderBuffer>,
    #[storage(119, read_only)]
    pub(super) vertex_probes: Handle<ShaderBuffer>,
    #[storage(120, read_only)]
    pub(super) shadow: Handle<ShaderBuffer>,
    #[storage(126, read_only)]
    pub(super) local_shadow: Handle<ShaderBuffer>,
    #[texture(127, dimension = "2d_array", sample_type = "depth")]
    #[sampler(128, sampler_type = "comparison")]
    pub(super) local_shadow_map: Option<Handle<Image>>,
    #[texture(121, dimension = "2d_array", sample_type = "depth")]
    #[sampler(122, sampler_type = "comparison")]
    pub(super) shadow_map: Option<Handle<Image>>,
    #[texture(123)]
    pub(super) screen_map: Option<Handle<Image>>,
    #[texture(124)]
    #[sampler(125)]
    pub(super) cloud_map: Option<Handle<Image>>,
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
    #[texture(142)]
    pub(super) lightmap: Option<Handle<Image>>,
    #[texture(143)]
    pub(super) filling_diffuse: Option<Handle<Image>>,
    #[texture(144)]
    pub(super) filling_distortion1: Option<Handle<Image>>,
    #[texture(145)]
    pub(super) filling_distortion2: Option<Handle<Image>>,
    #[texture(146)]
    pub(super) filling_lookup: Option<Handle<Image>>,
    #[texture(147)]
    pub(super) filling_normal: Option<Handle<Image>>,
    #[texture(148)]
    pub(super) filling_fresnel: Option<Handle<Image>>,
    #[texture(150)]
    #[sampler(149)]
    pub(super) linear_clamp_image: Option<Handle<Image>>,
    #[texture(151)]
    pub(super) cloud_opacity: Option<Handle<Image>>,
    #[texture(152)]
    pub(super) cloud_density: Option<Handle<Image>>,
    #[texture(153)]
    pub(super) cloud_back_density: Option<Handle<Image>>,
}

// Original common vertex shader transforms N/T/B independently with the
// model's linear rows. A separate bitangent avoids replacing that rule with
// inverse-transpose normals or a post-transform cross product under shear.
pub(super) const DH2_BITANGENT: MeshVertexAttribute =
    MeshVertexAttribute::new("DH2_Bitangent", 0x4448_3200, VertexFormat::Float32x3);
pub(super) const DH2_NORMAL_W: MeshVertexAttribute =
    MeshVertexAttribute::new("DH2_NormalW", 0x4448_3201, VertexFormat::Float32);

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
    if key.0[7]&24==24 && !descriptor.vertex.shader_defs.iter().any(
        |value| matches!(value,bevy::shader::ShaderDefVal::Bool(name,true) if name=="PREPASS_PIPELINE")
    ) {
        descriptor.vertex.shader_defs.push("DH2_FOG_VERTEX".into());
        if !descriptor.vertex.shader_defs.iter().any(
            |value| matches!(value,bevy::shader::ShaderDefVal::UInt(name,_) if name=="DH2_ALPHA_FLAGS")
        ) {
            descriptor.vertex.shader_defs.push(bevy::shader::ShaderDefVal::UInt("DH2_ALPHA_FLAGS".into(),key.0[7]));
        }
        if let Some(fragment)=descriptor.fragment.as_mut() {fragment.shader_defs.push("DH2_FOG_VERTEX".into());}
    }
    // Bevy omits opaque depth-only fragments unless its alpha-mask flag is
    // set. These original programs discard procedurally even with ONE/ZERO
    // blending: liquid clips its fill level; sky never contributes Z.
    if matches!(key.0[6], 6 | 7 | 8)
        && descriptor.fragment.is_none()
        && descriptor.vertex.shader_defs.iter().any(
            |v| matches!(v,bevy::shader::ShaderDefVal::Bool(name,true) if name=="PREPASS_PIPELINE"),
        )
    {
        descriptor.fragment = Some(bevy::render::render_resource::FragmentState {
            shader: PROBE_PREPASS_SHADER_HANDLE,
            shader_defs: descriptor.vertex.shader_defs.clone(),
            targets: Vec::new(),
            ..default()
        });
    }
    descriptor
        .vertex
        .shader_defs
        .push(bevy::shader::ShaderDefVal::UInt(
            "DH2_PROJECT_FAR".into(),
            key.0[15],
        ));
    // An absent project option must leave upstream materials compilable.
    // The composer evaluates nested numeric #if expressions even inside a
    // disabled #ifdef, so the mesh program uses a separate presence flag.
    if key.0[15] == 1 {
        descriptor.vertex.shader_defs.push(bevy::shader::ShaderDefVal::Bool(
            "DH2_FAR_DEPTH".into(), true,
        ));
    }
    if matches!(key.0[6], 7 | 8) {
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
        }
    }
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
    if layout.0.contains(DH2_NORMAL_W) {
        let extra = layout.0.get_layout(&[DH2_NORMAL_W.at_shader_location(9)])?;
        descriptor.vertex.buffers[0].attributes.extend(extra.attributes);
        descriptor.vertex.shader_defs.push("DH2_NORMAL_W".into());
    }
    Ok(())
}

pub(super) fn specialize_vertex_lighting(
    descriptor: &mut RenderPipelineDescriptor,
    layout: &MeshVertexBufferLayoutRef,
    key: surface_gpu::SurfaceKey,
) {
    let lighting = matches!(key.0[6], 1 | 2 | 6)
        && layout.0.contains(Mesh::ATTRIBUTE_NORMAL)
        && layout.0.contains(Mesh::ATTRIBUTE_TANGENT)
        && descriptor.fragment.as_ref().is_some_and(|fragment| {
            fragment.shader == PROBE_SHADER_HANDLE || fragment.shader == DECAL_SHADER_HANDLE
        });
    if lighting {
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

pub(super) fn depth_prepass_material_keys(
    material_free: bool,
) -> bevy::pbr::DepthPrepassMaterialKeys {
    let mut keys = bevy::pbr::DepthPrepassMaterialKeys::default();
    keys.register::<super::ProbeMaterial>(|mut key| {
        key.extension = key.extension.depth_prepass();
        key
    });
    if material_free {
        keys.register_material_free::<super::ProbeMaterial>(|key| {
            !key.extension.depth_needs_material()
        });
    }
    keys
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
            fragment.shader_defs.push("DH2_LOCAL_SHADOWS".into());
        }
        specialize_vertex_lighting(descriptor, layout, key.bind_group_data);
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
    #[storage(167, read_only)]
    pub(super) fog_objects: Handle<ShaderBuffer>,
    #[storage(161, read_only)]
    pub(super) fog_parameters: Handle<ShaderBuffer>,
    #[texture(162)]
    pub(super) fog_world: Option<Handle<Image>>,
    #[texture(163)]
    #[sampler(166)]
    pub(super) fog_canvas: Option<Handle<Image>>,
    #[texture(164)]
    pub(super) fog_curve: Option<Handle<Image>>,
    #[texture(165, sample_type = "depth")]
    pub(super) fog_depth: Option<Handle<Image>>,
    #[texture(159)]
    #[sampler(160)]
    pub(super) reflection_brdf_lookup: Option<Handle<Image>>,
    #[texture(156, dimension = "cube_array")]
    #[sampler(157)]
    pub(super) reflections: Option<Handle<Image>>,
    #[storage(158, read_only)]
    pub(super) reflection_planes: Handle<ShaderBuffer>,
    #[storage(154, read_only)]
    pub(super) material_exposure: Handle<ShaderBuffer>,
    #[uniform(155)]
    pub(super) bias: f32,
    #[storage(100, read_only)]
    pub(super) sh: Handle<ShaderBuffer>,
    #[storage(119, read_only)]
    pub(super) vertex_probes: Handle<ShaderBuffer>,
    #[storage(120, read_only)]
    pub(super) shadow: Handle<ShaderBuffer>,
    #[storage(126, read_only)]
    pub(super) local_shadow: Handle<ShaderBuffer>,
    #[texture(127, dimension = "2d_array", sample_type = "depth")]
    #[sampler(128, sampler_type = "comparison")]
    pub(super) local_shadow_map: Option<Handle<Image>>,
    #[texture(121, dimension = "2d_array", sample_type = "depth")]
    #[sampler(122, sampler_type = "comparison")]
    pub(super) shadow_map: Option<Handle<Image>>,
    #[texture(123)]
    pub(super) screen_map: Option<Handle<Image>>,
    #[texture(124)]
    #[sampler(125)]
    pub(super) cloud_map: Option<Handle<Image>>,
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
    #[texture(142)]
    pub(super) lightmap: Option<Handle<Image>>,
    #[texture(143)]
    pub(super) filling_diffuse: Option<Handle<Image>>,
    #[texture(144)]
    pub(super) filling_distortion1: Option<Handle<Image>>,
    #[texture(145)]
    pub(super) filling_distortion2: Option<Handle<Image>>,
    #[texture(146)]
    pub(super) filling_lookup: Option<Handle<Image>>,
    #[texture(147)]
    pub(super) filling_normal: Option<Handle<Image>>,
    #[texture(148)]
    pub(super) filling_fresnel: Option<Handle<Image>>,
    #[texture(150)]
    #[sampler(149)]
    pub(super) linear_clamp_image: Option<Handle<Image>>,
    #[texture(151)]
    pub(super) cloud_opacity: Option<Handle<Image>>,
    #[texture(152)]
    pub(super) cloud_density: Option<Handle<Image>>,
    #[texture(153)]
    pub(super) cloud_back_density: Option<Handle<Image>>,
}

pub(super) trait ImageAccess {
    fn image_mut(&mut self, role: surface_gpu::ImageRole) -> &mut Option<Handle<Image>>;
}
macro_rules! image_access {
    ($ty:ty) => {
        impl ImageAccess for $ty {
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
                    22 => &mut self.lightmap,
                    23 => &mut self.filling_diffuse,
                    24 => &mut self.filling_distortion1,
                    25 => &mut self.filling_distortion2,
                    26 => &mut self.filling_lookup,
                    27 => &mut self.filling_normal,
                    28 => &mut self.filling_fresnel,
                    29 => &mut self.cloud_opacity,
                    30 => &mut self.cloud_density,
                    31 => &mut self.cloud_back_density,
                    _ => panic!("invalid authored image role"),
                }
            }
        }
    };
}
image_access!(ProbeExtension);
image_access!(DecalExtension);

pub(super) trait ShadowImageAccess {
    fn sync_shadow_images(
        &mut self,
        target: &Option<Handle<Image>>,
        screen: &Option<Handle<Image>>,
        cloud: &Option<Handle<Image>>,
        local: &Option<Handle<Image>>,
    );
}
macro_rules! shadow_image_access {
    ($ty:ty) => {
        impl ShadowImageAccess for $ty {
            fn sync_shadow_images(
                &mut self,
                target: &Option<Handle<Image>>,
                screen: &Option<Handle<Image>>,
                cloud: &Option<Handle<Image>>,
                local: &Option<Handle<Image>>,
            ) {
                if &self.shadow_map != target {
                    self.shadow_map = target.clone();
                }
                if &self.screen_map != screen {
                    self.screen_map = screen.clone();
                }
                if &self.cloud_map != cloud {
                    self.cloud_map = cloud.clone();
                }
                if &self.local_shadow_map != local {
                    self.local_shadow_map = local.clone();
                }
            }
        }
    };
}
shadow_image_access!(ProbeExtension);
shadow_image_access!(DecalExtension);

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub(super) struct DecalKey {
    pub(super) surface: surface_gpu::SurfaceKey,
}

impl From<&DecalExtension> for DecalKey {
    fn from(extension: &DecalExtension) -> Self {
        Self {
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
        descriptor.vertex.shader_defs.push("DH2_DECAL".into());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push("DH2_LOCAL_SHADOWS".into());
        }
        specialize_vertex_lighting(descriptor, layout, key.bind_group_data.surface);
        specialize_surface(descriptor, key.bind_group_data.surface);
        authored_basis_pipeline(descriptor, layout)
    }
}
