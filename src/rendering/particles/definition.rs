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
    sync::{Arc, Weak},
};

fn check_active(cancelled: Option<&dyn Fn() -> bool>) -> Result<(), String> {
    if cancelled.is_some_and(|stop| stop()) { return Err("particle acquisition cancelled".into()); }
    Ok(())
}

/// Level-scoped fast lookup table, shared by player and world effects.
/// Catalog resource identities own declaration and PMODEL residency. The
/// original declarations/resources are immutable; only emitters own simulation
/// state. These host caches never share a particle's age, RNG or pool slot.
pub struct ParticleResources {
    images: super::image_resources::ParticleImages,
    pub(super) decal_resources: crate::assets::decal::DecalResources,
    declarations: HashMap<String, Result<Arc<particle_declaration::ParticleDeclaration>, String>>,
    models: HashMap<String, Arc<crate::assets::particle_model::ParticleModel>>,
    field_images: HashMap<String, Arc<crate::assets::fx::particle_field::FieldPixels>>,
    // The render cache may keep a material alive independently of its FX.
    // Two strong cache entries would each appear to have another consumer.
    materials: HashMap<String, Weak<ParticleMaterial>>,
    world_materials: HashMap<String, Result<bool, String>>,
    pub declaration_requests: usize,
    pub declaration_reads: usize,
    pub model_reads: usize,
    pub field_image_reads: usize,
    pub material_reads: usize,
    shared: bool,
    parameter_blocks: Option<Box<crate::assets::parm_block::ParmBlockSources>>,
}
impl Default for ParticleResources {
    fn default() -> Self {
        Self {
            images: Default::default(),
            decal_resources: Default::default(),
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
            parameter_blocks: None,
        }
    }
}
impl ParticleResources {
    pub fn use_scene_images(&mut self) { self.images.use_scene_images(); }
    pub fn scene_image(&mut self, catalog: &Catalog, name: &str) -> Result<Option<Arc<super::image_resources::ParticleImage>>, String> {
        self.images.scene_image(catalog, name)
    }
    pub fn images(&self) -> impl Iterator<Item = Arc<super::image_resources::ParticleImage>> + '_ {
        self.images.images()
    }

    /// Acquisition caches cannot promote an otherwise unused asset into GPU
    /// residency. Runtime declarations/materials own their required backing.
    pub fn retire_unused(&mut self) {
        let before = (self.declarations.len(), self.materials.len(), self.models.len(), self.field_images.len());
        // 0607f0 reuses registered identities, including failed loads. Do not
        // turn retirement into a retry loop for immutable source failures.
        self.declarations.retain(|_, value| match value {
            Ok(value) => Arc::strong_count(value) > 1,
            Err(_) => true,
        });
        self.materials.retain(|_, value| value.strong_count() != 0);
        self.models.retain(|_, value| Arc::strong_count(value) > 1);
        self.field_images.retain(|_, value| Arc::strong_count(value) > 1);
        let (images, payload_bytes) = self.images.retire_unused();
        if self.parameter_blocks.as_mut().is_some_and(|blocks| blocks.retire_unused()) {
            self.parameter_blocks = None;
        }
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
    fn model(&mut self, catalog: &Catalog, resource: &str, marker: u8, cancelled: Option<&dyn Fn() -> bool>) -> Result<(), String> {
        check_active(cancelled)?;
        if self.models.contains_key(resource) { return Ok(()); }
        let model = crate::assets::fx::particle_resources::model(catalog, resource, marker, self.shared, cancelled, || {
            self.model_reads += 1;
            crate::assets::particle_model::load(catalog, resource, cancelled)?
                .map(Arc::new).ok_or_else(|| format!("missing particle model {resource}"))
        })?;
        check_active(cancelled)?;
        self.models.insert(resource.to_owned(), model);
        Ok(())
    }
    fn declaration(
        &mut self,
        catalog: &Catalog,
        path: &str,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Arc<particle_declaration::ParticleDeclaration>, String> {
        check_active(cancelled)?;
        self.declaration_requests += 1;
        let key = path.replace('\\', "/").to_ascii_lowercase();
        if self.shared {
            if let Some(loaded) = self.declarations.get(&key) {
                return loaded.clone();
            }
        }
        let loaded = crate::assets::fx::particle_resources::declaration(catalog, path, self.shared, cancelled, |marker| {
            self.declaration_reads += 1;
            let _tag = crate::platform::allocation_tags::tag("particle definitions");
            // Preserve archive IO failures distinctly from absent declarations.
            let mut read_error = None;
            let mut read = |path: &str| match catalog.read_cancellable(path, cancelled) {
                Ok(bytes) => bytes,
                Err(error) => { read_error = Some(format!("{path}: {error}")); None }
            };
            let fields = read_edit(&mut read, "particledt", path);
            if let Some(error) = read_error { return Err(error); }
            let fields = fields?;
            check_active(cancelled)?;
            let mut declaration = particle_declaration::declaration(&fields)
                .map_err(|error| format!("{path}: {error}"))?;
            // Emission meshes are dedicated PMODEL archive resources,
            // independent of the ordinary rendered-prop mesh loader.
            for stage in &mut declaration.stages {
                check_active(cancelled)?;
                for field in stage
                    .spawn_fields
                    .iter_mut()
                    .chain(&mut stage.vector_fields)
                    .chain(&mut stage.acceleration_fields)
                {
                    for texture in &mut field.textures {
                        check_active(cancelled)?;
                        let Some(source) = &texture.source else {
                            continue;
                        };
                        let resource = catalog
                            .image_resource(source)
                            .ok_or_else(|| format!("missing field image alias {source}"))?;
                        if !self.field_images.contains_key(resource) {
                            let pixels = crate::assets::fx::particle_resources::field(catalog, resource, self.shared, cancelled, || {
                                self.field_image_reads += 1;
                                let bytes = catalog.read_cancellable(resource, cancelled).map_err(|error| format!("{resource}: {error}"))?
                                    .ok_or_else(|| format!("missing field image {resource}"))?;
                                Ok(Arc::new(crate::assets::fx::particle_field::FieldPixels::read(&bytes)
                                    .map_err(|error| format!("{resource}: {error}"))?))
                            })?;
                            check_active(cancelled)?;
                            self.field_images.insert(resource.to_owned(), pixels);
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
                    self.model(catalog, &resource, marker, cancelled)?;
                    *model = Some(self.models[&resource].clone());
                }
                for mesh in &mut stage.meshes {
                    let Some(source) = &mesh.source else { continue };
                    let resource = crate::assets::particle_model::resource_path(source)?;
                    self.model(catalog, &resource, marker, cancelled)?;
                    mesh.model = Some(self.models[&resource].clone());
                }
            }

            Ok(Arc::new(declaration))
        });
        check_active(cancelled)?;
        self.declarations.insert(key, loaded.clone());
        loaded
    }
}

/// Exercise the real declaration owner with a stop after an installed member
/// was read, then reacquire in the same catalog and worker cache.
#[cfg(feature = "game")]
pub(crate) fn observe_cancelled_declaration(base: &std::path::Path, name: &str)
    -> Result<(), Box<dyn std::error::Error>> {
    let catalog = Catalog::open(base)?;
    let path = format!("generated/decls/particledt/{name}.decl");
    if catalog.kind(&path) != Some("particledt") || catalog.has_overlay_file(&path) {
        return Err("expected an installed particle declaration".into());
    }
    let cancelled = || crate::loading::catalog_read_recorded(catalog.identity(), "particledt", &path);
    let mut resources = ParticleResources::default();
    let stopped = resources.declaration(&catalog, name, Some(&cancelled))
        .is_err_and(|error| error.contains("acquisition cancelled"));
    let reached_source = cancelled();
    let unpublished = resources.declarations.is_empty();
    let reads_before_retry = resources.declaration_reads;
    let first = resources.declaration(&catalog, name, None)?;
    let read_again = resources.declaration_reads == reads_before_retry + 1;
    let second = resources.declaration(&catalog, name, None)?;
    let reused = Arc::ptr_eq(&first, &second);
    let stages = first.stages.len();
    let observed = Arc::downgrade(&first);
    drop((first, second, resources));
    catalog.release_level_resources();
    let retired = observed.upgrade().is_none() && !cancelled();
    eprintln!("PARTICLE_CANCELLED_DECLARATION name={name} stages={stages} stopped={stopped} reached_source={reached_source} unpublished={unpublished} read_again={read_again} reused={reused} retired={retired}");
    Ok(())
}

#[derive(Clone)]
pub struct ParticleDefinition {
    pub owner: Arc<particle_system::ParticleSystemAsset>,
    /// The immutable authored block is shared; mutable per-instance values
    /// remain with the runtime FX owner (140227cd0, 140228110).
    pub(crate) parameter_block: Option<Arc<crate::assets::parm_block::ParmBlock>>,
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
        Self::load_cancellable(fields, catalog, library, resources, None)
    }

    pub(super) fn load_cancellable(
        fields: &[(String, Value)], catalog: &Catalog, library: &mut ProgramLibrary,
        resources: &mut ParticleResources, cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, String> {
        check_active(cancelled)?;
        let owner = particle_system::particle_system(fields)?;
        let parameter_block = owner.parameter_block.as_deref().map(|name| {
            resources.parameter_blocks
                .get_or_insert_with(|| Box::new(crate::assets::parm_block::ParmBlockSources::new(catalog)))
                .acquire(catalog, name, |parameter| library.parameter_value_metadata(catalog, parameter))
        }).transpose()?;
        if !resources.shared || catalog.has_overlays() {
            // Existing emitters retain their Arc generation. Mutable source
            // requests must not reuse an earlier decoded payload or failure.
            resources.declarations.clear();
            resources.models.clear();
            resources.field_images.clear();
            resources.materials.clear();
            resources.world_materials.clear();
        }
        let declarations = owner
            .declarations
            .iter()
            .map(|path| {
                path.as_deref()
                    .map(|path| resources.declaration(catalog, path, cancelled))
                    .transpose()
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut materials = HashMap::new();
        let mut world_mesh_materials = HashSet::new();
        for declaration in declarations.iter().flatten() {
            for stage in declaration.stages.iter().filter(|stage| stage.enabled) {
                for mesh in stage.meshes.iter().filter(|mesh| mesh.enabled) {
                    check_active(cancelled)?;
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
                    check_active(cancelled)?;
                    let Some(name) = material else {
                        continue;
                    };
                    if world_mesh_materials.contains(name) {
                        continue;
                    }
                    let key = name.replace('\\', "/").to_ascii_lowercase();
                    let cached = resources.materials.get(&key).and_then(Weak::upgrade);
                    let material = if let Some(material) = cached {
                        material
                    } else {
                        resources.material_reads += 1;
                        let material = Arc::new(ParticleMaterial::load_shared(
                            catalog, library, name, &mut resources.images,
                        )?);
                        resources.materials.insert(key, Arc::downgrade(&material));
                        material
                    };
                    materials.insert(name.clone(), material);
                }
            }
        }
        check_active(cancelled)?;
        Ok(Self {
            owner: Arc::new(owner),
            parameter_block,
            declarations,
            materials,
            world_mesh_materials,
        })
    }
}
