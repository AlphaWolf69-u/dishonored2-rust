use crate::{
    archive::Catalog,
    assets::fx::screen_quad::{screen_quad, ScreenQuadStage},
    gameplay::{
        effects::{environment::EnvironmentEffectDefinition, timing::EffectTimingRanges},
        options::decl::{atom, list_items, read_edit, scope},
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
    pub groups: Vec<GroupDefinition>,
}
pub struct ScreenEffectSources {
    pub swimming: crate::gameplay::player::swimming::SwimmingSettings,
    pub underwater: EffectDefinition,
    pub leave_water: EffectDefinition,
    pub materials: Vec<ScreenDistortionMaterial>,
    pub resolve_state: crate::render_state::RenderState,
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
        let declarations = crate::entities::player::player_declarations(character, |path| {
            catalog
                .read(path)
                .map(|b| b.map(|b| crate::entities::declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        })?;
        let swimming = scope(&declarations.fx, "m_swimming")
            .ok_or("player FX manager swimming descriptor absent")?;
        let underwater = atom(swimming, "m_underWaterFX").ok_or("player underwater FX absent")?;
        let leave_water = atom(swimming, "m_leaveWaterFX").ok_or("player leave-water FX absent")?;
        let mut materials = Vec::new();
        let mut indices = HashMap::<String, usize>::new();
        let mut particle_materials = HashMap::new();
        let mut load = |effect: &str| -> Result<EffectDefinition, String> {
            let mut read = |p: &str| catalog.read(p).ok().flatten();
            let fields = read_edit(&mut read, "fx", effect)?;
            let mut groups = Vec::new();
            for group in scope(&fields, "m_groups").into_iter().flat_map(list_items) {
                let crate::entities::Value::Scope(fields) = group else {
                    continue;
                };
                let common = crate::assets::fx::environment_group(fields)?;
                if !common.enabled {
                    continue;
                }
                if common.location != crate::assets::fx::EffectGroupLocation::Everywhere {
                    return Err(format!(
                        "screen effect group location is not connected: {:?}",
                        common.location
                    ));
                }
                let mut quads = Vec::new();
                for quad in scope(fields, "m_fullScreenQuads")
                    .into_iter()
                    .flat_map(list_items)
                {
                    let crate::entities::Value::Scope(fields) = quad else {
                        continue;
                    };
                    let quad = screen_quad(fields)?;
                    if !quad.enabled || quad.stage == ScreenQuadStage::Never {
                        continue;
                    }
                    let Some(name) = quad.material else { continue };
                    if !matches!(
                        quad.stage,
                        ScreenQuadStage::MotionVectorBegin | ScreenQuadStage::MotionVectorEnd
                    ) {
                        return Err(format!(
                            "underwater FX requires unsupported screen stage {:?}",
                            quad.stage
                        ));
                    }
                    let material = if let Some(&index) = indices.get(&name) {
                        index
                    } else {
                        let index = materials.len();
                        materials.push(ScreenDistortionMaterial::load(catalog, library, &name)?);
                        indices.insert(name, index);
                        index
                    };
                    quads.push(QuadDefinition {
                        material,
                        stage: quad.stage,
                        blend: Arc::new(quad.blend),
                    });
                }
                groups.push(GroupDefinition {
                    timing: common.timings,
                    quads,
                    particles: scope(fields, "m_particleSystems")
                        .into_iter()
                        .flat_map(list_items)
                        .filter(|value| match value {
                            crate::entities::Value::Scope(fields) => {
                                atom(fields, "m_enabled") != Some("false")
                            }
                            _ => true,
                        })
                        .map(|value| match value {
                            crate::entities::Value::Scope(fields) => ParticleDefinition::load(
                                fields,
                                catalog,
                                library,
                                &mut particle_materials,
                            ),
                            _ => Err("particle FX descriptor requires a scope".into()),
                        })
                        .collect(),
                });
            }
            Ok(EffectDefinition { groups })
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
            transparency_resolve_state,
        })
    }
}
