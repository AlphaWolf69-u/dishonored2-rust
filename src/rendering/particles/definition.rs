//! Installed particle definitions and shared material preparation. Both player
//! and world FX use this loader; playback ownership stays with their caller.
use super::material::ParticleMaterial;
use crate::{
    archive::Catalog,
    assets::{
        entities::Value,
        fx::{particle_declaration, particle_system},
    },
    gameplay::options::decl::read_edit,
    shader_cache::ProgramLibrary,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// Level-scoped acquisition owner, shared by player and world effects. The
/// original declarations/resources are immutable; only emitters own simulation
/// state. These host caches never share a particle's age, RNG or pool slot.
pub struct ParticleResources {
    images: super::image_resources::ParticleImages,
    declarations: HashMap<String, Result<Arc<particle_declaration::ParticleDeclaration>, String>>,
    models: HashMap<String, Arc<crate::assets::particle_model::ParticleModel>>,
    field_images: HashMap<String, Arc<crate::assets::fx::particle_field::FieldPixels>>,
    materials: HashMap<String, Arc<ParticleMaterial>>,
    world_materials: HashMap<String, Result<bool, String>>,
    pub declaration_requests: usize,
    pub declaration_reads: usize,
    pub model_reads: usize,
    pub field_image_reads: usize,
    pub material_reads: usize,
    shared: bool,
}
impl Default for ParticleResources {
    fn default() -> Self {
        Self {
            images: Default::default(),
            declarations: Default::default(),
            models: Default::default(),
            field_images: Default::default(),
            materials: Default::default(),
            world_materials: Default::default(),
            declaration_requests: 0,
            declaration_reads: 0,
            model_reads: 0,
            field_image_reads: 0,
            material_reads: 0,
            shared: true,
        }
    }
}
impl ParticleResources {
    pub fn use_scene_images(&mut self) { self.images.use_scene_images(); }
    pub fn scene_image(&mut self, catalog: &Catalog, name: &str) -> Result<Option<Arc<super::image_resources::ParticleImage>>, String> {
        self.images.scene_image(catalog, name)
    }
    pub fn images(&self) -> impl Iterator<Item = &Arc<super::image_resources::ParticleImage>> {
        self.images.images()
    }

    /// Acquisition caches cannot promote an otherwise unused asset into GPU
    /// residency. Runtime declarations/materials own their required backing.
    pub fn retire_unused(&mut self) {
        let before = (self.declarations.len(), self.materials.len(), self.models.len(), self.field_images.len());
        self.declarations.retain(|_, value| value.as_ref().is_ok_and(|value| Arc::strong_count(value) > 1));
        self.materials.retain(|_, value| Arc::strong_count(value) > 1);
        self.models.retain(|_, value| Arc::strong_count(value) > 1);
        self.field_images.retain(|_, value| Arc::strong_count(value) > 1);
        let (images, payload_bytes) = self.images.retire_unused();
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY particle cache retirement: declarations={} materials={} models={} fields={} images={} acquisition_payload_bytes={}",
                before.0 - self.declarations.len(), before.1 - self.materials.len(),
                before.2 - self.models.len(), before.3 - self.field_images.len(), images, payload_bytes);
        }
    }

    pub fn report_scene_images(&self, paths: &[String]) { self.images.report_scene_overlap(paths); }

    /// Diagnostic acquisition A/B; production always shares immutable assets.
    pub fn debug_set_sharing(&mut self, shared: bool) {
        self.shared = shared;
    }
    fn declaration(
        &mut self,
        catalog: &Catalog,
        path: &str,
    ) -> Result<Arc<particle_declaration::ParticleDeclaration>, String> {
        self.declaration_requests += 1;
        let key = path.replace('\\', "/").to_ascii_lowercase();
        if self.shared {
            if let Some(loaded) = self.declarations.get(&key) {
                return loaded.clone();
            }
        }
        self.declaration_reads += 1;
        let _tag = crate::platform::allocation_tags::tag("particle definitions");
        let loaded = (|| {
            let mut read = |p: &str| catalog.read(p).ok().flatten();
            let fields = read_edit(&mut read, "particledt", path)?;
            let mut declaration = particle_declaration::declaration(&fields)
                .map_err(|error| format!("{path}: {error}"))?;
            // Emission meshes are dedicated PMODEL archive resources,
            // independent of the ordinary rendered-prop mesh loader.
            for stage in &mut declaration.stages {
                for field in stage
                    .spawn_fields
                    .iter_mut()
                    .chain(&mut stage.vector_fields)
                    .chain(&mut stage.acceleration_fields)
                {
                    for texture in &mut field.textures {
                        let Some(source) = &texture.source else {
                            continue;
                        };
                        let resource = catalog
                            .image_resource(source)
                            .ok_or_else(|| format!("missing field image alias {source}"))?;
                        if !self.field_images.contains_key(resource) {
                            self.field_image_reads += 1;
                            let bytes = catalog
                                .read(resource)
                                .map_err(|error| format!("{resource}: {error}"))?
                                .ok_or_else(|| format!("missing field image {resource}"))?;
                            self.field_images.insert(
                                resource.to_owned(),
                                Arc::new(
                                    crate::assets::fx::particle_field::FieldPixels::read(&bytes)
                                        .map_err(|error| format!("{resource}: {error}"))?,
                                ),
                            );
                        }
                        texture.pixels = Some(self.field_images[resource].clone());
                    }
                }
                for (source, model) in [
                    (&stage.spawn_mesh.source, &mut stage.spawn_mesh.model),
                    (&stage.velocity_mesh.source, &mut stage.velocity_mesh.model),
                ] {
                    let Some(source) = source else { continue };
                    let resource = crate::assets::particle_model::resource_path(source)?;
                    if !self.models.contains_key(&resource) {
                        self.model_reads += 1;
                        let bytes = catalog
                            .read(&resource)
                            .map_err(|error| format!("{resource}: {error}"))?
                            .ok_or_else(|| format!("missing particle emission mesh {resource}"))?;
                        self.models.insert(
                            resource.clone(),
                            Arc::new(
                                crate::assets::particle_model::parse(&bytes)
                                    .map_err(|error| format!("{resource}: {error}"))?,
                            ),
                        );
                    }
                    *model = Some(self.models[&resource].clone());
                }
                for mesh in &mut stage.meshes {
                    let Some(source) = &mesh.source else { continue };
                    let resource = crate::assets::particle_model::resource_path(source)?;
                    if !self.models.contains_key(&resource) {
                        self.model_reads += 1;
                        let bytes = catalog
                            .read(&resource)
                            .map_err(|error| format!("{resource}: {error}"))?
                            .ok_or_else(|| format!("missing particle render mesh {resource}"))?;
                        self.models.insert(
                            resource.clone(),
                            Arc::new(
                                crate::assets::particle_model::parse(&bytes)
                                    .map_err(|error| format!("{resource}: {error}"))?,
                            ),
                        );
                    }
                    mesh.model = Some(self.models[&resource].clone());
                }
            }

            Ok(Arc::new(declaration))
        })();
        self.declarations.insert(key, loaded.clone());
        loaded
    }
}

#[derive(Clone)]
pub struct ParticleDefinition {
    pub owner: Arc<particle_system::ParticleSystemAsset>,
    pub declarations: Vec<Option<Arc<particle_declaration::ParticleDeclaration>>>,
    pub materials: HashMap<String, Arc<ParticleMaterial>>,
    /// Mesh materials whose MAIN program is a world program. 1401fbad0 makes
    /// every renderer an ordinary model surface (material = desc+0x278), so
    /// these draw through the world material's own passes, not the FX pass.
    pub world_mesh_materials: HashSet<String>,
}
impl ParticleDefinition {
    pub fn load(
        fields: &[(String, Value)],
        catalog: &Catalog,
        library: &mut ProgramLibrary,
        resources: &mut ParticleResources,
    ) -> Result<Self, String> {
        let owner = particle_system::particle_system(fields)?;
        if !resources.shared {
            resources.models.clear();
            resources.field_images.clear();
        }
        let declarations = owner
            .declarations
            .iter()
            .map(|path| {
                path.as_deref()
                    .map(|path| resources.declaration(catalog, path))
                    .transpose()
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut materials = HashMap::new();
        let mut world_mesh_materials = HashSet::new();
        for declaration in declarations.iter().flatten() {
            for stage in declaration.stages.iter().filter(|stage| stage.enabled) {
                for mesh in stage.meshes.iter().filter(|mesh| mesh.enabled) {
                    if let Some(name) = &mesh.material {
                        let key = name.replace('\\', "/").to_ascii_lowercase();
                        if *resources
                            .world_materials
                            .entry(key)
                            .or_insert_with(|| super::material::world_program(catalog, name))
                            .as_ref()
                            .map_err(Clone::clone)?
                        {
                            world_mesh_materials.insert(name.clone());
                        }
                    }
                }
                let names = stage
                    .sprites
                    .iter()
                    .filter(|sprite| sprite.enabled)
                    .map(|sprite| &sprite.material)
                    .chain(
                        stage
                            .meshes
                            .iter()
                            .filter(|mesh| mesh.enabled)
                            .map(|mesh| &mesh.material),
                    );
                for material in names {
                    let Some(name) = material else {
                        continue;
                    };
                    if world_mesh_materials.contains(name) {
                        continue;
                    }
                    let key = name.replace('\\', "/").to_ascii_lowercase();
                    if !resources.materials.contains_key(&key) {
                        resources.material_reads += 1;
                        resources.materials.insert(
                            key.clone(),
                            Arc::new(ParticleMaterial::load_shared(catalog, library, name, &mut resources.images)?),
                        );
                    }
                    materials.insert(name.clone(), resources.materials[&key].clone());
                }
            }
        }
        Ok(Self {
            owner: Arc::new(owner),
            declarations,
            materials,
            world_mesh_materials,
        })
    }
}
