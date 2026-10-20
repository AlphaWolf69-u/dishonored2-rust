//! PostFX_Sun_Sprite: the environment's textured celestial disc, at far depth.
use super::{
    entities, environment_sun_ray, game_to_view, material, texture_streaming, SceneData,
    DEFAULT_FAR,
};
use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::*,
    shader::ShaderRef,
};

const SHADER: Handle<Shader> = bevy::asset::uuid_handle!("b43a6ae4-53e2-4cb3-99f3-7c9f3a034b56");
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(SpriteBlend)]
pub(super) struct CelestialSprite {
    #[uniform(0)]
    position_size: Vec4,
    #[uniform(1)]
    color: Vec4,
    #[texture(2)]
    #[sampler(3)]
    image: Handle<Image>,
    additive: bool,
}
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(super) struct SpriteBlend(bool);
impl From<&CelestialSprite> for SpriteBlend {
    fn from(s: &CelestialSprite) -> Self {
        Self(s.additive)
    }
}
impl Material for CelestialSprite {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        // The original celestial disc is an HDR postFX pass after scene
        // drawing. This is sorting only; its tested depth remains zero.
        f32::NEG_INFINITY
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
        }
        if key.bind_group_data.0 {
            if let Some(fragment) = descriptor.fragment.as_mut() {
                for target in fragment.targets.iter_mut().flatten() {
                    target.blend = Some(BlendState {
                        color: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::One,
                            operation: BlendOperation::Add,
                        },
                        alpha: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::One,
                            operation: BlendOperation::Add,
                        },
                    });
                }
            }
        }
        Ok(())
    }
}
pub(super) fn register(app: &mut App) {
    register_renderer(app);
    // Level setup transfers the archive catalog into the texture worker.
    app.add_systems(Startup, spawn.before(super::setup));
}
pub(super) fn register_renderer(app: &mut App) {
    app.add_plugins(MaterialPlugin::<CelestialSprite>::default());
    bevy::asset::load_internal_asset!(
        app,
        SHADER,
        "../../assets/shaders/dh2_sun_sprite.wgsl",
        Shader::from_wgsl
    );
}
#[cfg(test)]
pub(super) fn comparison_material(image: Handle<Image>) -> CelestialSprite {
    CelestialSprite {
        position_size: Vec4::new(0., 0., -6300., 0.06),
        color: Vec4::new(0.001, 0.0005, 0.00025, 1.),
        image,
        additive: true,
    }
}
fn spawn(
    mut commands: Commands,
    scene: Res<SceneData>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CelestialSprite>>,
) {
    if let Err(error) = spawn_authored(
        &mut commands,
        &scene,
        &mut images,
        &mut meshes,
        &mut materials,
    ) {
        eprintln!("Celestial sprite unavailable: {error}");
    }
}
fn spawn_authored(
    commands: &mut Commands,
    scene: &SceneData,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<CelestialSprite>,
) -> Result<(), String> {
    let Some(p) = scene.environment_render_parameters.as_ref() else {
        return Ok(());
    };
    let scalar = |name, default| p.scalar_parameter(name).map(|v| v.unwrap_or(default));
    if scalar("atm/worldfog/usesuntexture", 0.)? == 0.0 {
        return Ok(());
    }
    let size = scalar("atm/worldfog/sunspritesize", 0.)? * 0.01; // FUN_14030a800,141ab3810.
    if size <= 0.0 {
        return Ok(());
    }
    let direction = if scalar("atm/worldfog/spriteusesundirection", 0.)? != 0.0 {
        -environment_sun_ray(
            scene
                .environment_lighting
                .ok_or("sun environment unavailable")?,
        )
        .ok_or("sun direction unavailable")?
    } else {
        game_to_view(crate::render_shadow::environment_sun_direction(
            scalar("atm/worldfog/sunspriteangletheta", 45.)?,
            scalar("atm/worldfog/sunspriteanglephi", 0.)?,
        ))
    };
    let text = p
        .parameter("atm/worldfog/sunspritecolor")
        .unwrap_or("{54,52.8,45,1}");
    let color =
        entities::material_float_parameters(&format!("{{ value {text} }}"), |n| n == "value")?[0]
            .1
            .map(f32::from_bits);
    let catalog = scene.catalog.as_ref().ok_or("catalog unavailable")?;
    let name = p
        .parameter("atm/worldfog/suntexture")
        .ok_or("sun texture unresolved")?
        .trim_matches('"');
    let stem = material::compiled_image_stem(catalog, name)
        .ok_or_else(|| format!("sun image {name} absent"))?;
    let mut streamed = texture_streaming::read_streamed_diffuse_at_target(
        catalog,
        &stem,
        texture_streaming::TextureReadTarget::Initial(Some(0)),
    )?
    .ok_or("sun image payload absent")?;
    streamed.image.data = Some(
        streamed
            .uploads
            .into_iter()
            .flat_map(|m| m.payload)
            .collect(),
    );
    streamed.image.sampler =
        bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
            address_mode_u: bevy::image::ImageAddressMode::ClampToEdge,
            address_mode_v: bevy::image::ImageAddressMode::ClampToEdge,
            mag_filter: bevy::image::ImageFilterMode::Linear,
            min_filter: bevy::image::ImageFilterMode::Linear,
            mipmap_filter: bevy::image::ImageFilterMode::Linear,
            lod_max_clamp: 13.,
            ..default()
        });
    let image = images.add(streamed.image);
    let material = materials.add(CelestialSprite {
        position_size: (direction * DEFAULT_FAR * 0.9).extend(size),
        color: Vec4::from_array(color),
        image,
        additive: scalar("atm/worldfog/sunblendadditive", 1.)? != 0.0,
    });
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(2., 2.))),
        MeshMaterial3d(material),
        Transform::default(),
        bevy::camera::visibility::NoFrustumCulling,
        bevy::light::NotShadowCaster,
        bevy::light::NotShadowReceiver,
    ));
    eprintln!("[sky] celestial sprite texture={name} size={size} direction={direction:?}");
    Ok(())
}
