//! PlayerFXFocusedAttack (81fd60/81fda0/81fe00), with the installed
//! cpntPlayerFXManager declaration and the shared21a240 environment producer.
//! The separate NPC-driven CombatFocus81f7e0 is not this predicate.
use crate::{archive::Catalog, entities::{Value, scene_motion::{atom, child}},
    gameplay::effects::{environment::{EnvironmentBlendEffect, EnvironmentEffectDefinition},
        groups::sample_offsets, timing::EffectTimingRanges}};
use std::sync::Arc;

struct Environment {
    definition: Arc<EnvironmentEffectDefinition>,
    offsets: EffectTimingRanges,
    values: [Option<f32>; 33],
}
pub struct Definition {
    _source: Arc<crate::assets::fx::definition::Definition>,
    environments: Vec<Environment>,
}
impl Definition {
    pub(crate) fn acquire(catalog: &Catalog, fields: &[(String, Value)]) -> Result<Option<Arc<Self>>, String> {
        let Some(path) = atom(child(fields, "m_focusedAttack"), "m_screenFX")
            .filter(|name| !name.is_empty() && *name != "NULL") else { return Ok(None) };
        let source = crate::assets::fx::declaration::acquire(catalog, path,
            crate::assets::fx::definition::Consumer::Common, None)?;
        let mut environments = Vec::new();
        for group in &source.groups {
            let group = &group.asset;
            if !group.enabled { continue; }
            for environment in &group.environments {
                if !environment.enabled { continue; }
                let Some(name) = &environment.environment else { continue };
                let read = |name: &str| -> Result<Option<String>, String> {
                    catalog.read(&format!("generated/decls/env/{name}.decl"))
                        .map(|bytes| bytes.map(crate::entities::owned_declaration_text)).map_err(|error| error.to_string())
                };
                let text = read(name)?.ok_or_else(|| format!("player combat environment {name} is absent"))?;
                let parameters = crate::entities::environment_render_parameters_with_inheritance(name, &text, read)?;
                let reader = crate::assets::environment::ParameterReader { catalog, parameters: &parameters };
                let mut values = [None; 33];
                for (slot, name) in crate::rendering::camera_effects::PARAMETER_NAMES.iter().enumerate() {
                    if parameters.parameter(name).is_some() { values[slot] = Some(reader.vector(name)?[0]); }
                }
                environments.push(Environment { definition: environment.blend.clone(), offsets: group.timings, values });
            }
        }
        Ok(Some(Arc::new(Self { _source: source, environments })))
    }
}

struct Active { definition: usize, producer: EnvironmentBlendEffect, time: f32, stopping: bool }
pub struct Runtime {
    definition: Arc<Definition>,
    active: Vec<Active>,
    was_active: bool,
    seed: u32,
}
impl Runtime {
    pub fn new(definition: Arc<Definition>) -> Self {
        let active = Vec::with_capacity(definition.environments.len());
        Self { definition, active, was_active: false, seed: 0 }
    }
    pub fn advance(&mut self, active: bool, delta: f32) -> Result<(), String> {
        if active && !self.was_active {
            for (index, definition) in self.definition.environments.iter().enumerate() {
                let offsets = sample_offsets(&definition.offsets, 0., &mut self.seed);
                self.active.push(Active { definition: index,
                    producer: EnvironmentBlendEffect::new(definition.definition.clone(), offsets, 1, &mut self.seed)?,
                    time: 0., stopping: false });
            }
        }
        for effect in &mut self.active {
            if !active && !effect.stopping { effect.producer.stop(effect.time); effect.stopping = true; }
            effect.time += delta;
            effect.producer.update(effect.time, 1, &mut self.seed)?;
        }
        self.active.retain(|effect| effect.producer.active);
        self.was_active = active;
        Ok(())
    }
    pub fn apply(&self, slot: usize, value: &mut f32) {
        for active in &self.active {
            if let Some(target) = self.definition.environments[active.definition].values[slot] {
                *value += (target - *value) * active.producer.blend.clamp(0., 1.);
            }
        }
    }
}
