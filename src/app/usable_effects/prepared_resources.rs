//! Host material readiness depends on immutable inputs, not playing actors.
//! These are weak identities only; actual source/material owners retain data.
use super::Sources;
use std::{collections::{HashMap, HashSet}, sync::{Arc, Weak}};

#[derive(Clone, Default)]
pub(super) struct Inputs {
    particles: HashMap<usize, (String, Weak<crate::rendering::particles::material::ParticleMaterial>)>,
    quads: HashMap<(usize,u32), Weak<crate::rendering::screen_quad::ScreenQuadMaterial>>,
    strips: HashSet<String>,
    meshes: HashMap<(String, String), Option<Weak<crate::assets::particle_model::ParticleModel>>>,
    decals: HashSet<(String, bool)>,
}
impl Inputs {
    fn capture(source: &Sources) -> Self {
        let mut particles = HashMap::new();
        for material in source.particle_materials() {
            particles.entry(Arc::as_ptr(material) as usize)
                .or_insert_with(|| (material.name.clone(), Arc::downgrade(material)));
        }
        Self {
            quads: source.quad_material_stages().map(|(stage,material)|((Arc::as_ptr(material) as usize,stage as u32),Arc::downgrade(material))).collect(),
            particles, strips: source.strip_materials(),
            meshes: source.world_meshes().into_iter().map(|(name, material, model)| {
                //world_meshes constructs an empty placeholder for the SAME
                //code-owned sprite-grid level on every enumeration. Its
                //canonical identity is that level, not this temporary Arc.
                let grid = name.strip_prefix("sprite-grid:").and_then(|level| level.parse::<u32>().ok());
                ((name, material), grid.is_none().then(|| Arc::downgrade(&model)))
            }).collect(),
            decals: source.decal_materials().into_iter().collect(),
        }
    }
    fn covered_by(&self, prepared: &Self) -> bool {
        self.quads.iter().all(|(key,identity)|prepared.quads.get(key).is_some_and(|other|Weak::ptr_eq(identity,other)))
            && self.particles.iter().all(|(key, (name, identity))| prepared.particles.get(key)
            .is_some_and(|(other_name, other)| name == other_name && Weak::ptr_eq(identity, other)))
            && self.strips.is_subset(&prepared.strips)
            && self.meshes.iter().all(|(key, identity)| prepared.meshes.get(key)
                .is_some_and(|other| match (identity, other) {
                        (None, None) => true,
                        (Some(identity), Some(other)) => Weak::ptr_eq(identity, other),
                        _ => false,
                    }))
            && self.decals.is_subset(&prepared.decals)
    }
    fn same(&self, other: &Self) -> bool { self.covered_by(other) && other.covered_by(self) }
}

#[derive(Default)]
pub(super) struct PreparedResources {
    sources: Vec<Weak<Sources>>,
    by_source: HashMap<usize, Inputs>,
    observed_sources: HashSet<usize>,
    current: Option<Inputs>,
    acknowledged: Option<Inputs>,
    pub generation: u64,
    ///Counters are scalars; formatting occurs only in the existing F8 path.
    pub changed: u64,
    pub reused: u64,
    force_reack: bool,
    bindings: Option<crate::app::runtime_materials::PreparedBindings>,
    decal_bindings: Option<crate::app::projected_decals::PreparedProxyBindings>,
    acknowledged_generation: u64,
    acknowledged_bindings: (Option<u64>, Option<u64>),
}
impl PreparedResources {
    pub fn set_control(&mut self, force_reack: bool) { self.force_reack = force_reack; }
    ///Only called after the desired/ready source revision actually changes.
    pub fn sources_changed<'a>(&mut self, sources: impl Iterator<Item = &'a Arc<Sources>>) -> bool {
        let mut count = 0;
        self.observed_sources.clear();
        for source in sources {
            let identity = Arc::as_ptr(source) as usize;
            if !self.observed_sources.insert(identity) { continue; }
            count += 1;
            if !self.by_source.contains_key(&identity) { return true; }
        }
        count != self.sources.len()
    }
    pub fn update<'a>(&mut self, sources: impl Iterator<Item = &'a Arc<Sources>>, dynamic: &Sources, force_reack: bool) -> bool {
        self.force_reack = force_reack;
        self.sources.clear(); self.by_source.clear();
        for source in sources {
            let identity = Arc::as_ptr(source) as usize;
            if self.by_source.contains_key(&identity) { continue; }
            self.sources.push(Arc::downgrade(source));
            self.by_source.insert(identity, Inputs::capture(source));
        }
        let inputs = Inputs::capture(dynamic);
        let changed = force_reack || self.current.as_ref().is_none_or(|previous| !previous.same(&inputs));
        if changed {
            self.current = Some(inputs);
            self.generation = self.generation.wrapping_add(1).max(1);
            self.changed += 1;
        } else { self.reused += 1; }
        changed
    }
    pub fn acknowledge(&mut self, coordinator: Option<&crate::app::runtime_materials::RuntimeMaterialCoordinator>,
        decals: Option<&crate::app::projected_decals::RuntimeDecalProxies>) {
        let revisions = (coordinator.map(|owner| owner.binding_revision()), decals.map(|owner| owner.binding_revision()));
        if self.acknowledged_generation == self.generation && self.acknowledged_bindings == revisions { return; }
        self.acknowledged_generation = self.generation;
        self.acknowledged_bindings = revisions;
        self.acknowledged.clone_from(&self.current);
        self.bindings = self.current.as_ref().and_then(|inputs| coordinator.map(|owner| owner.capture_bindings(
            inputs.meshes.keys().map(|(name, material)| (name, material)), inputs.decals.iter())));
        self.decal_bindings = self.current.as_ref().and_then(|inputs| decals.map(|owner| owner.capture_bindings(inputs.decals.iter())));
    }
    pub fn covers(&self, source: &Arc<Sources>) -> bool {
        !self.force_reack && self.by_source.get(&(Arc::as_ptr(source) as usize))
            .zip(self.acknowledged.as_ref()).is_some_and(|(needed, ready)| needed.covered_by(ready))
    }
    pub fn needs_world(&self) -> bool { self.current.as_ref().is_some_and(|inputs| !inputs.meshes.is_empty()) }
    pub fn needs_decals(&self) -> bool { self.current.as_ref().is_some_and(|inputs| !inputs.decals.is_empty()) }
    pub fn bindings_match(&self, source: &Arc<Sources>, coordinator: Option<&crate::app::runtime_materials::RuntimeMaterialCoordinator>,
        decals: Option<&crate::app::projected_decals::RuntimeDecalProxies>) -> bool {
        let Some(inputs) = self.by_source.get(&(Arc::as_ptr(source) as usize)) else { return false; };
        inputs.meshes.keys().all(|key| coordinator.zip(self.bindings.as_ref())
            .is_some_and(|(owner, bindings)| owner.mesh_binding_matches(bindings, key)))
            && inputs.decals.iter().all(|key| coordinator.zip(self.bindings.as_ref())
                .is_some_and(|(owner, bindings)| owner.decal_binding_matches(bindings, key))
                && (coordinator.is_some_and(|owner| owner.static_decal_binding(key))
                    || decals.zip(self.decal_bindings.as_ref())
                        .is_some_and(|(owner, bindings)| owner.binding_matches(bindings, key))))
    }
}
