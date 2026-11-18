//! PlayerFXFocusedAttack (81fd60/81fda0/81fe00), with the installed
//! cpntPlayerFXManager declaration and the shared21a240 environment producer.
//! The separate NPC-driven CombatFocus81f7e0 is not this predicate.
use crate::{
    archive::Catalog,
    entities::{
        scene_motion::{atom, child},
        Value,
    },
    gameplay::effects::{
        environment::{EnvironmentBlendEffect, EnvironmentEffectDefinition},
        groups::sample_offsets,
        timing::EffectTimingRanges,
    },
};
use std::sync::Arc;

///684680/6846b0: this is a shared attached FX, separate from the focused
///environment producer below. The shared FX owner decodes its FSDisto tracks.
pub(crate) fn motion_blur_source(fields: &[(String, Value)]) -> Option<Arc<str>> {
    atom(child(fields, "m_motionBlur"), "m_screenFX")
        .filter(|path| !path.is_empty() && *path != "NULL")
        .map(Arc::from)
}

struct Environment {
    environment: Arc<crate::rendering::environment_resources::PreparedEnvironment>,
    definition: Arc<EnvironmentEffectDefinition>,
    offsets: EffectTimingRanges,
    values: [Option<f32>; 33],
}
pub struct Definition {
    _source: Arc<crate::assets::fx::definition::Definition>,
    environments: Vec<Environment>,
}
impl Definition {
    pub(crate) fn acquire(
        catalog: &Catalog,
        fields: &[(String, Value)],
    ) -> Result<Option<Arc<Self>>, String> {
        let Some(path) = atom(child(fields, "m_focusedAttack"), "m_screenFX")
            .filter(|name| !name.is_empty() && *name != "NULL")
        else {
            return Ok(None);
        };
        let source = crate::assets::fx::declaration::acquire(
            catalog,
            path,
            crate::assets::fx::definition::Consumer::Common,
            None,
        )?;
        let mut environments = Vec::new();
        for group in &source.groups {
            let group = &group.asset;
            if !group.enabled {
                continue;
            }
            for environment in &group.environments {
                if !environment.enabled {
                    continue;
                }
                let Some(name) = &environment.environment else {
                    continue;
                };
                let prepared =
                    crate::rendering::environment_resources::PreparedEnvironment::acquire(
                        catalog, name, 0,
                    )?;
                let mut values = [None; 33];
                for (slot, name) in crate::rendering::camera_effects::PARAMETER_NAMES.iter().enumerate() {
                    values[slot] = match prepared.values.get(*name) {
                        //313e70's environment camera inputs are registered floating
                        //uniforms. Reuse the exact acquisition-time float words;
                        //never silently discard or reinterpret another numeric type.
                        Some(crate::rendering::environment_resources::PreparedValue::Float(value)) => Some(value[0]),
                        None => None,
                        Some(_) => return Err(format!("focused-attack camera parameter {name} is not a floating uniform")),
                    };
                }
                environments.push(Environment {
                    environment: prepared,
                    definition: environment.blend.clone(),
                    offsets: group.timings,
                    values,
                });
            }
        }
        Ok(Some(Arc::new(Self {
            _source: source,
            environments,
        })))
    }
}

struct Active {
    definition: usize,
    producer: EnvironmentBlendEffect,
    time: f32,
    stopping: bool,
    binding: crate::rendering::environment_effects::EnvironmentEffectBinding<
        Arc<crate::rendering::environment_resources::PreparedEnvironment>,
        u64,
    >,
    lease: crate::rendering::environment_effect_runtime::EnvironmentPoolLease,
}
/// On-demand view of the actual producer; no copied definition, diagnostic
/// clock, retained frame history, or work in ordinary gameplay updates.
pub struct DiagnosticSnapshot<'a> {
    pub definition: usize,
    pub definition_identity: usize,
    pub authored: &'a EnvironmentEffectDefinition,
    pub parent_timing: &'a EffectTimingRanges,
    pub producer: &'a EnvironmentBlendEffect,
    pub time: f32,
    pub stopping: bool,
    pub targets: &'a [Option<f32>; 33],
}
pub struct Runtime {
    definition: Arc<Definition>,
    active: Vec<Active>,
    was_active: bool,
    seed: u32,
    pool: Option<crate::rendering::environment_effect_runtime::SharedEnvironmentPool>,
    retirements: Vec<(u64, Option<usize>)>,
    closed: bool,
}
impl Runtime {
    pub fn environment_identity(&self) -> usize {
        Arc::as_ptr(&self.definition) as usize
    }
    pub fn environment_pool_bound(&self) -> bool {
        self.pool.is_some()
    }
    pub fn focus_requested(&self) -> bool {
        self.was_active
    }

    pub fn definition_sources(&self) -> &[String] {
        &self.definition._source.source_paths
    }

    pub fn visit_diagnostics(&self, mut visit: impl FnMut(DiagnosticSnapshot<'_>)) {
        for active in &self.active {
            let definition = &self.definition.environments[active.definition];
            visit(DiagnosticSnapshot {
                definition: active.definition,
                definition_identity: Arc::as_ptr(&definition.definition) as usize,
                authored: &definition.definition,
                parent_timing: &definition.offsets,
                producer: &active.producer,
                time: active.time,
                stopping: active.stopping,
                targets: &definition.values,
            });
        }
    }

    pub fn new(definition: Arc<Definition>) -> Self {
        let active = Vec::with_capacity(definition.environments.len());
        Self {
            definition,
            active,
            was_active: false,
            seed: 0,
            pool: None,
            retirements: Vec::new(),
            closed: false,
        }
    }
    pub fn advance(&mut self, active: bool, delta: f32) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        let mut index = 0;
        while index < self.active.len() {
            if self.active[index].producer.active {
                index += 1;
                continue;
            }
            self.active[index].lease.release()?;
            let effect = self.active.remove(index);
            if let Some((view, slot)) = effect.binding.into_registration() {
                self.retirements.push((view, Some(slot)));
            }
        }
        if active && !self.was_active {
            if self.definition.environments.iter().any(|e| {
                !e.environment
                    .prepared
                    .load(std::sync::atomic::Ordering::Acquire)
            }) {
                return Ok(());
            }
            let pool = self
                .pool
                .as_ref()
                .ok_or("focused environment has no canonical pool binding")?;
            for (index, definition) in self.definition.environments.iter().enumerate() {
                let offsets = sample_offsets(&definition.offsets, 0., &mut self.seed);
                let Some(lease) = pool.acquire_external()? else {
                    continue;
                };
                self.active.push(Active {
                    definition: index,
                    producer: EnvironmentBlendEffect::new(
                        definition.definition.clone(),
                        offsets,
                        1,
                        &mut self.seed,
                    )?,
                    time: 0.,
                    stopping: false,
                    lease,
                    binding: crate::rendering::environment_effects::EnvironmentEffectBinding::new(
                        definition.environment.clone(),
                    ),
                });
            }
        }
        for effect in &mut self.active {
            if !active && !effect.stopping {
                effect.producer.stop(effect.time);
                effect.stopping = true;
            }
            effect.time += delta;
            effect.producer.update(effect.time, 1, &mut self.seed)?;
        }
        self.was_active = active;
        Ok(())
    }
    pub fn set_environment_pool(
        &mut self,
        pool: crate::rendering::environment_effect_runtime::SharedEnvironmentPool,
    ) -> Result<(), String> {
        if self.pool.as_ref().is_some_and(|old| old.is_same(&pool)) {
            return Ok(());
        }
        if !self.active.is_empty() {
            return Err("focused environment pool changed while records remain live".into());
        }
        self.pool = Some(pool);
        Ok(())
    }
    pub fn initialize_environment_pool(
        &mut self,
        pool: crate::rendering::environment_effect_runtime::SharedEnvironmentPool,
    ) -> Result<(), String> {
        if !self.active.is_empty() || !self.retirements.is_empty() {
            return Err("previous focused environment generation is still owned".into());
        }
        self.set_environment_pool(pool)?;
        self.closed = false;
        Ok(())
    }
    pub fn visit_environment_sources(
        &self,
        mut visit: impl FnMut(&Arc<crate::rendering::environment_resources::PreparedEnvironment>),
    ) {
        for source in &self.definition.environments {
            visit(&source.environment);
        }
    }
    pub fn visit_environment_orders(&self, mask: u8, mut visit: impl FnMut(u64)) {
        for effect in &self.active {
            if effect.producer.view_mask & mask != 0 {
                visit(effect.producer.registration_order);
            }
        }
    }
    pub fn publish_environment(
        &mut self,
        order: u64,
        view: u64,
        mask: u8,
        slots: &mut crate::rendering::environment_overrides::EnvironmentOverrides<
            Arc<crate::rendering::environment_resources::PreparedEnvironment>,
        >,
    ) -> Result<(), String> {
        let effect = self
            .active
            .iter_mut()
            .find(|effect| effect.producer.registration_order == order)
            .ok_or("focused environment registration disappeared")?;
        effect.binding.publish(
            view,
            slots,
            effect.producer.view_mask,
            mask,
            effect.producer.blend,
        )
    }
    pub fn flush_environment_retirements(
        &mut self,
        view: u64,
        slots: &mut crate::rendering::environment_overrides::EnvironmentOverrides<
            Arc<crate::rendering::environment_resources::PreparedEnvironment>,
        >,
    ) {
        for (owner, slot) in self.retirements.drain(..) {
            if owner == view {
                slots.retire(slot);
            }
        }
    }
    pub fn clear_environments(&mut self) -> Result<(), String> {
        self.closed = true;
        while !self.active.is_empty() {
            self.active[0].lease.release()?;
            let effect = self.active.remove(0);
            if let Some((view, slot)) = effect.binding.into_registration() {
                self.retirements.push((view, Some(slot)));
            }
        }
        self.was_active = false;
        Ok(())
    }
    pub fn apply(&self, slot: usize, value: &mut f32) {
        //21a190 ->806a00 ->890970 registers {blend, influence=1}.
        //88e220 accumulates participating targets and blend weights, then
        //normalizes both before ONE base-to-target interpolation. Retriggered
        //focus can overlap the previous effect's authored fade-out; sequential
        //lerps would compound those envelopes instead of averaging them.
        let mut target_sum = 0.;
        let mut blend_sum = 0.;
        let mut influence = 0.;
        for active in &self.active {
            if let Some(target) = self.definition.environments[active.definition].values[slot] {
                target_sum += target;
                blend_sum += active.producer.blend;
                influence += 1.;
            }
        }
        if influence > 0. {
            let inverse = 1. / influence;
            *value += (target_sum * inverse - *value) * (blend_sum * inverse).clamp(0., 1.);
        }
    }
}
