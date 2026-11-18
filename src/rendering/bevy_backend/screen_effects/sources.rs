use crate::{
    archive::Catalog,
    assets::fx::screen_quad::ScreenQuadStage,
    gameplay::{
        effects::{environment::EnvironmentEffectDefinition, timing::EffectTimingRanges},
        options::decl::{atom, scope},
    },
    rendering::screen_distortion::ScreenDistortionMaterial,
    shader_cache::ProgramLibrary,
};
use std::sync::Arc;

pub struct QuadDefinition {
    pub material: usize,
    pub stage: ScreenQuadStage,
    pub blend: Arc<EnvironmentEffectDefinition>,
}
pub struct GroupDefinition {
    pub timing: EffectTimingRanges,
    pub quads: Vec<QuadDefinition>,
    /// Particle readiness is independent of screen-distortion readiness.
    /// Retain descriptor errors for the particle owner without disabling quads.
    pub particles: Vec<Result<ParticleDefinition, String>>,
}
pub use crate::rendering::particles::definition::ParticleDefinition;
pub struct EffectDefinition {
    pub(crate) _source: Arc<crate::assets::fx::definition::Definition>,
    pub groups: Vec<GroupDefinition>,
}
/// darkVisionDepthWaveColor (POSTPROC 140327270) and darkVisionDepthWaveDisto
/// (DISTORTION 1403222f0): their authored states and registered parameters.
pub struct DarkVisionWaveSources {
    pub colour_state: crate::render_state::RenderState,
    pub distortion_state: crate::render_state::RenderState,
    /// (noise power, colour noise power, colour power, 0), edge thickness
    /// (x, y), tint, edge tint.
    pub parameters: [[f32; 4]; 4],
}

pub struct ScreenEffectSources {
    pub swimming: crate::gameplay::player::swimming::SwimmingSettings,
    pub underwater: EffectDefinition,
    pub leave_water: EffectDefinition,
    pub materials: Vec<ScreenDistortionMaterial>,
    pub resolve_state: crate::render_state::RenderState,
    pub fullscreen_state: crate::render_state::RenderState,
    pub transparency_resolve_state: Result<crate::render_state::RenderState, String>,
    /// The Dark Vision sonar wave programs (rendering/darkVisionDepthWave*).
    pub dark_vision: DarkVisionWaveSources,
    /// rendering/darkvision state and the registered layer constants.
    pub dark_vision_layers: crate::rendering::bevy_backend::dark_vision_layers::DarkVisionLayerSources,
}
impl ScreenEffectSources {
    /// Every particle material of the player screen effects.
    pub fn particle_materials(&self) -> impl Iterator<Item = &std::sync::Arc<crate::rendering::particles::material::ParticleMaterial>> {
        [&self.underwater, &self.leave_water].into_iter().flat_map(|e| &e.groups).flat_map(|g| &g.particles)
            .filter_map(|p| p.as_ref().ok()).flat_map(|p| p.materials.values())
    }

    pub fn load(
        catalog: &Catalog,
        library: &mut ProgramLibrary,
        character: &str,
    ) -> Result<Self, String> {
        Self::load_shared(catalog, library, character, &mut Default::default())
    }

    pub fn load_shared(
        catalog: &Catalog,
        library: &mut ProgramLibrary,
        character: &str,
        resources: &mut crate::rendering::particles::definition::ParticleResources,
    ) -> Result<Self, String> {
        Self::load_shared_with_player(catalog, library, character, resources, &mut Default::default())
    }

    pub(crate) fn load_shared_with_player(
        catalog: &Catalog,
        library: &mut ProgramLibrary,
        character: &str,
        resources: &mut crate::rendering::particles::definition::ParticleResources,
        declarations_source: &mut crate::entities::player::DeclarationSource,
    ) -> Result<Self, String> {
        let declarations = declarations_source.screen(catalog, character)?;
        let swimming = scope(&declarations.fx, "m_swimming")
            .ok_or("player FX manager swimming descriptor absent")?;
        let underwater = atom(swimming, "m_underWaterFX").ok_or("player underwater FX absent")?;
        let leave_water = atom(swimming, "m_leaveWaterFX").ok_or("player leave-water FX absent")?;
        // Generic WorldFX now owns the complete underwater/leave-water
        // groups, their particles and canonical screen-quad image residency.
        // Retain only declaration metadata here during the compatibility
        // handoff; no second quad/material/image or particle preparation.
        let materials = Vec::new();
        let load = |effect: &str| -> Result<EffectDefinition,String> {
            let source=crate::assets::fx::declaration::acquire(catalog,effect,
                crate::assets::fx::definition::Consumer::Screen,None)?;
            Ok(EffectDefinition{_source:source,groups:Vec::new()})
        };
        let underwater = load(underwater)?;
        let leave_water = load(leave_water)?;
        let bytes = catalog
            .read("generated/decls/material/postfx/motionblurdisto.material.decl")
            .map_err(|e| e.to_string())?
            .ok_or("motion distortion material absent")?;
        let main = crate::material::authored_program_override(&bytes, "mainProgram")
            .ok_or("motion distortion MAIN absent")?;
        let mut resolve_state = crate::render_state::material_render_state(
            &crate::entities::declaration_text(&bytes),
            library
                .definition(main.as_bytes())?
                .ok_or("motion distortion definition absent")?
                .render_state(),
        )?;
        // 326930's full-screen overwrite, distinct from the material's blend.
        resolve_state[0] = (resolve_state[0] & 0xffe0_0803) | 0x802;
        let transparency_resolve_state = library
            .definition(b"PostFX_CompositeWeightedBlendedOIT")
            .and_then(|definition| {
                definition
                    .map(|d| d.render_state())
                    .ok_or_else(|| "weighted transparency resolve definition absent".into())
            });
        let empty = crate::render_program::SerializedParameterBlock { flags: 0, parameters: Vec::new() };
        let mut parameter = |name: &str| -> Result<[f32; 4], String> {
            Ok(library.registered_float_vector(catalog, &empty, name.as_bytes(), None)
                .map_err(|e| format!("{name}: {e}"))?.map(f32::from_bits))
        };
        let thickness = parameter("rendering/darkVisionDepthWaveEdgeThickness")?;
        let device_tint = parameter("rendering/darkVisionDeviceTint")?;
        let pickup_tint = parameter("rendering/darkVisionPickupTint")?;
        let layer_tints = [
            [device_tint[0], device_tint[1], device_tint[2], parameter("rendering/darkVisionDeviceOpacity")?[0]],
            [pickup_tint[0], pickup_tint[1], pickup_tint[2], parameter("rendering/darkVisionPickupOpacity")?[0]],
        ];
        let layer_depth_bias = parameter("rendering/darkVisionDepthBias")?[0];
        let parameters = [
            [parameter("rendering/darkVisionDepthWaveNoisePower")?[0],
                parameter("rendering/darkVisionDepthWaveColorNoisePower")?[0],
                parameter("rendering/darkVisionDepthWaveColorPower")?[0], 0.],
            [thickness[0], thickness[1], 0., 0.],
            parameter("rendering/darkVisionDepthWaveTint")?,
            parameter("rendering/darkVisionDepthWaveEdgeTint")?,
        ];
        let mut state = |name: &str| -> Result<crate::render_state::RenderState, String> {
            Ok(library.definition(name.as_bytes())?.ok_or_else(|| format!("{name} definition absent"))?.render_state())
        };
        let dark_vision_layers = crate::rendering::bevy_backend::dark_vision_layers::DarkVisionLayerSources {
            colour_state: state("rendering/darkvision")?,
            tints: layer_tints,
            depth_bias: layer_depth_bias,
        };
        let dark_vision = DarkVisionWaveSources {
            colour_state: state("rendering/darkVisionDepthWaveColor")?,
            distortion_state: state("rendering/darkVisionDepthWaveDisto")?,
            parameters,
        };
        Ok(Self {
            swimming: crate::gameplay::player::swimming::SwimmingSettings::read(
                &declarations.controller,
            )?,
            underwater,
            leave_water,
            materials,
            resolve_state,
            fullscreen_state: library.definition(b"PostFX_uberMotionVector")?
                .ok_or("fullscreen distortion program definition absent")?.render_state(),
            transparency_resolve_state,
            dark_vision,
            dark_vision_layers,
        })
    }
}
