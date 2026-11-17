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
use std::{collections::HashMap, sync::Arc};

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
pub struct ScreenEffectSources {
    pub swimming: crate::gameplay::player::swimming::SwimmingSettings,
    pub underwater: EffectDefinition,
    pub leave_water: EffectDefinition,
    pub materials: Vec<ScreenDistortionMaterial>,
    pub resolve_state: crate::render_state::RenderState,
    pub fullscreen_state: crate::render_state::RenderState,
    pub transparency_resolve_state: Result<crate::render_state::RenderState, String>,
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
        let mut materials = Vec::new();
        let mut indices = HashMap::<String, usize>::new();
        let mut load = |effect: &str| -> Result<EffectDefinition, String> {
            let source = crate::assets::fx::declaration::acquire(catalog, effect,
                crate::assets::fx::definition::Consumer::Screen, None)?;
            let mut groups = Vec::new();
            for group in &source.groups {
                let common = &group.asset;
                if !common.enabled { continue; }
                if common.location != crate::assets::fx::EffectGroupLocation::Everywhere {
                    return Err(format!(
                        "screen effect group location is not connected: {:?}",
                        common.location
                    ));
                }
                let mut quads = Vec::new();
                for quad in group.quads.get().expect("screen FX families acquired")
                    .as_ref().map_err(Clone::clone)? {
                    let quad = quad.as_ref().map_err(Clone::clone)?;
                    if !quad.enabled || quad.stage == ScreenQuadStage::Never {
                        continue;
                    }
                    let Some(name) = &quad.material else { continue };
                    if !matches!(
                        quad.stage,
                        ScreenQuadStage::MotionVectorBegin | ScreenQuadStage::MotionVectorEnd
                    ) {
                        return Err(format!(
                            "underwater FX requires unsupported screen stage {:?}",
                            quad.stage
                        ));
                    }
                    let material = if let Some(&index) = indices.get(name) {
                        index
                    } else {
                        let index = materials.len();
                        materials.push(ScreenDistortionMaterial::load(catalog, library, name)?);
                        indices.insert(name.clone(), index);
                        index
                    };
                    quads.push(QuadDefinition {
                        material,
                        stage: quad.stage,
                        blend: quad.blend.clone(),
                    });
                }
                groups.push(GroupDefinition {
                    timing: common.timings,
                    quads,
                    particles: match group.particles.get().expect("screen FX families acquired") {
                        Ok(particles) => particles.iter().map(|asset|
                            asset.as_ref().map_err(Clone::clone).and_then(|asset|
                                ParticleDefinition::from_asset_cancellable(asset.clone(), catalog, library, resources, None)))
                            .collect(),
                        Err(error) => vec![Err(error.clone())],
                    },
                });
            }
            Ok(EffectDefinition { _source: source, groups })
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
        })
    }
}
