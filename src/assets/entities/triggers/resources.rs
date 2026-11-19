//! Owned trigger shape files. Item references remain local to their HKX;
//! resource indices here are not runtime body IDs or Havok item references.
use super::{trigger_placements, Document, TriggerPlacement};
use crate::assets::{
    archive::Catalog,
    entities::declaration_text,
    physworld,
};
use std::collections::HashMap;

fn trace_collision_sources() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("DH2_DEBUG_COLLISION_SOURCES").is_some())
}

#[derive(Hash, PartialEq, Eq)]
struct StandaloneKey { source: usize, aliases: Vec<u64>, extra: Option<(usize, Vec<u64>, [u16; 2])> }

pub struct TriggerShapeAsset {
    pub path: String,
    // The shared source is retired only after cached_mass has been filled.
    // Runtime geometry queries use query_shapes.
    source: std::sync::Arc<physworld::source::ShapeSource>,
    /// Actual acquired wrapper resource leases for an admitted model body.
    standalone: Option<Box<physworld::source::StandaloneShape>>,
    /// The extra-HD child shares this asset's body lifetime.
    _standalone_extra: Option<Box<physworld::source::StandaloneShape>>,
    standalone_key: Option<std::sync::Arc<StandaloneKey>>,
    /// Serialized item identity in `source`; level systems need not use item 1.
    root: usize,
    /// Future body admissions can still reference names and mass in this source.
    retain_for_admission: bool,
    pub header: physworld::shape::ShapeHeader,
    pub bounds: [[f32; 4]; 2],
    pub is_mutable: bool,
    pub query_shapes: std::sync::Arc<physworld::query_geometry::QueryShapes>,
    cached_mass: Option<Result<Option<physworld::mass::PrincipalMassProperties>, String>>,
    pub(crate) signals: crate::gameplay::shape_registry::ShapeSignals,
    ///08b830 propertykey1 belongs to this actual shape generation. Both wind
    ///and water borrow it; the final geometry lease drops it automatically.
    physics_property_present: bool,
    physics_property: std::sync::OnceLock<Result<physworld::properties::generated::PhysicsShapeProperty,String>>,
}

impl std::ops::Deref for TriggerShapeAsset {
    type Target = physworld::source::ShapeSource;
    fn deref(&self) -> &Self::Target { &self.source }
}

impl crate::gameplay::shape_registry::ShapeSignalStore for TriggerAssets {
    fn signals_mut(&mut self,shape:usize)->Result<&mut crate::gameplay::shape_registry::ShapeSignals,String>{
        let shape=self.shapes.get_mut(shape).and_then(Option::as_mut).ok_or("trigger shape signal resource is missing")?;
        if !shape.is_mutable {return Err("immutable trigger shape has no mutation signals".into());}
        Ok(&mut shape.signals)
    }
}

impl crate::gameplay::physics_world::body_creation::BodyCreationGeometry for TriggerAssets {
    fn creation_assets(&self,info:&physworld::body::BodyInfo)
        ->Result<crate::gameplay::body_storage::BodyCreationAssets<'_>,String>{
        let shape=self.shapes.get(info.shape).and_then(Option::as_ref).ok_or("trigger creation shape is missing")?;
        Ok(crate::gameplay::body_storage::BodyCreationAssets { shape:shape.header,
            wrapped_shapes:None,
            name:shape.physics.body_name(&shape.bytes, info.name)?,
            mass_distribution:match info.mass_distribution_lease.as_deref(){Some(value)=>Some(*value),None=>shape.physics.body_mass_distribution(&shape.bytes,info.mass_distribution)?} })
    }
    fn header(&self,shape:usize)->Result<physworld::shape::ShapeHeader,String>{
        Ok(self.shapes.get(shape).and_then(Option::as_ref).ok_or("trigger shape header is missing")?.header)
    }
    fn is_mutable(&self,shape:usize)->Result<bool,String>{
        Ok(self.shapes.get(shape).and_then(Option::as_ref).ok_or("trigger shape mutability resource is missing")?.is_mutable)
    }
    fn rotation_limit_measure(&self,shape:usize)->Result<f32,String>{
        let shape=self.shapes.get(shape).and_then(Option::as_ref).ok_or("trigger rotation-limit shape is missing")?;
        shape.query_shapes.rotation_limit_measure(shape.query_shapes.root()).map_err(|error|
            format!("{} root={} type={} flags={:#x}: {error}",shape.path,shape.query_shapes.root(),shape.header.shape_type,shape.header.flags))
    }
    fn cached_mass(&self,shape:usize)->Result<Option<physworld::mass::PrincipalMassProperties>,String>{
        let shape=self.shapes.get(shape).and_then(Option::as_ref).ok_or("trigger mass shape is missing")?;
        shape.root_mass()
    }
    fn calculate_mass(&self,shape:usize,config:[u32;3])->Result<physworld::mass::PrincipalMassProperties,String>{
        let shape=self.shapes.get(shape).and_then(Option::as_ref).ok_or("trigger calculated-mass shape is missing")?;
        match &shape.query_shapes.get(shape.query_shapes.root())?.geometry {
            physworld::query_geometry::QueryGeometry::Compound(_) =>
                crate::gameplay::motion_info::calculate_compound_mass(config, shape.bounds),
            physworld::query_geometry::QueryGeometry::Polytope(polytope) =>
                physworld::mass::convex::calculate(&polytope.vertices, polytope.radius, config, shape.bounds),
            physworld::query_geometry::QueryGeometry::Convex(convex) =>
                if shape.header.shape_type == 2 {
                    Ok(physworld::mass::sphere_mass_properties(config,
                        *convex.vertices.first().ok_or("sphere mass centre absent")?, convex.radius, shape.bounds))
                } else {
                    physworld::mass::convex::calculate(&convex.vertices, convex.radius, config, shape.bounds)
                },
            physworld::query_geometry::QueryGeometry::CompressedMesh(_) =>
                Err("compressed world mesh has no dynamic mass calculator".into()),
        }.map_err(|error| format!("{}: {error}", shape.path))
    }
}

impl TriggerShapeAsset {
    pub(crate) fn root_index(&self) -> usize { self.query_shapes.root() }
    pub(crate) fn prepare_physics_property(&mut self, mesh:bool)->Result<(),String>{
        //A later mesh admission cannot erase key1 already installed on the
        //same actual shared shape; a later nonmesh admission may create it.
        if mesh { if self.physics_property.get().is_none(){self.physics_property_present=false;} }
        else if self._standalone_extra.is_none(){self.physics_property_present=true;}
        if self.physics_property_present { self.physics_property_ref()?; }
        Ok(())
    }
    pub(crate) fn physics_property_if_present(&self)->Result<Option<&physworld::properties::generated::PhysicsShapeProperty>,String>{
        if self.physics_property_present { self.physics_property_ref().map(Some) } else { Ok(None) }
    }
    pub(crate) fn physics_property_ref(&self)->Result<&physworld::properties::generated::PhysicsShapeProperty,String>{
        if !self.physics_property_present { return Err("shape has no generated key1 physics property".into()); }
        self.physics_property.get_or_init(||physworld::properties::generated::build(&self.query_shapes))
            .as_ref().map_err(Clone::clone)
    }
    fn root_mass(&self) -> Result<Option<physworld::mass::PrincipalMassProperties>, String> {
        if let Some(mass)=self.standalone.as_ref().and_then(|shape|shape.mass.as_ref()){
            //808ea0/08c7f0 overrides the actual root property, including
            //scaled aliases and the primary+ExtraHD combined compound.
            return Ok(**mass);
        }
        if self.standalone.as_ref().is_some_and(|shape| !shape.wrappers.is_empty()) {
            // 14008c9a0 constructs a new scaled compound without copying the
            // child's 0xf100 property. Its native mode-zero calculator uses
            // that wrapper's bounds instead of the unscaled child's mass.
            return Ok(None);
        }
        self.cached_mass.clone().unwrap_or_else(|| self.physics.shape_mass_properties(&self.bytes, self.root))
    }
    /// Prepare the motion from this retained compound resource's real cached
    /// mass or its original mode-zero calculation. World activation owns IDs.
    pub fn prepare_motion(
        &self,
        body: &physworld::body::BodyInfo,
        distribution: Option<physworld::body::MassDistribution>,
    ) -> Result<crate::gameplay::motion_info::MotionInfo, String> {
        let cached = self.root_mass()?;
        crate::gameplay::motion_info::MotionInfo::from_compound_body(
            body,
            cached,
            distribution,
            self.bounds,
        )
    }
}

pub struct TriggerAssets {
    pub placements: Vec<Option<TriggerPlacement>>,
    /// Parallel to placements, selecting the shape asset for each trigger.
    pub shape_resources: Vec<Option<usize>>,
    pub shapes: Vec<Option<TriggerShapeAsset>>,
    free_placements: Vec<usize>,
    free_shapes: Vec<usize>,
    shape_references: Vec<usize>,
    standalone_keys: HashMap<std::sync::Arc<StandaloneKey>, usize>,
}

impl crate::gameplay::physics_world::PublicationGeometry for TriggerAssets {
    fn is_null(&mut self, shape: usize) -> Result<bool, String> {
        // Every retained asset has a real root. Zero is a valid resource index;
        // an absent resource is an error, never the source's null pointer state.
        self.shapes.get(shape).and_then(Option::as_ref).ok_or_else(|| format!("missing trigger shape resource {shape}"))?;
        Ok(false)
    }

    fn bounds(
        &mut self, shape: usize, frame: crate::gameplay::shape_transform::ShapeMatrix,
    ) -> Result<[[f32; 4]; 2], String> {
        let shape = self.shapes.get(shape).and_then(Option::as_ref).ok_or_else(|| format!("missing trigger shape resource {shape}"))?;
        shape.query_shapes.bounds(shape.query_shapes.root(), frame)
    }
}

impl TriggerAssets {
    /// 852560 admits the level physics system on this same geometry owner.
    /// Keep serialized item references local to the resource and give body
    /// creation its canonical shape slot. Shared roots have one decoded tree.
    pub(crate) fn register_world_shape(&mut self, path: &str,
        source: std::sync::Arc<physworld::source::ShapeSource>, root: usize) -> Result<usize, String> {
        if let Some((index,_)) = self.shapes.iter().enumerate().find(|(_,shape)|
            shape.as_ref().is_some_and(|shape| shape.standalone.is_none() && shape.retain_for_admission
                && shape.root == root && std::sync::Arc::ptr_eq(&shape.source,&source))) {
            self.shape_references[index] = self.shape_references[index].checked_add(1).ok_or("world shape reference overflow")?;
            return Ok(index);
        }
        let query_shapes = source.query_shapes(root)?;
        let header = query_shapes.get(root)?.header;
        let is_mutable = match &query_shapes.get(root)?.geometry {
            physworld::query_geometry::QueryGeometry::Compound(shape) => shape.is_mutable,
            _ => false,
        };
        let identity = [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]];
        let bounds = query_shapes.bounds(root,identity)?;
        let asset = TriggerShapeAsset { path: path.to_owned(), source, standalone: None,
            _standalone_extra: None, standalone_key: None, root, retain_for_admission: true,
            header, bounds, is_mutable, query_shapes: std::sync::Arc::new(query_shapes),
            cached_mass: None, signals: Default::default(), physics_property_present: false, physics_property: Default::default() };
        self.shapes.try_reserve(1).map_err(|error| error.to_string())?;
        self.shape_references.try_reserve(1).map_err(|error| error.to_string())?;
        let index = self.shapes.len();
        self.shapes.push(Some(asset));
        self.shape_references.push(1);
        Ok(index)
    }

    pub(crate) fn retire_world_shape(&mut self, index: usize) -> Result<(),String> {
        let shape = self.shapes.get(index).and_then(Option::as_ref).ok_or("world shape absent")?;
        if shape.standalone.is_some() || !shape.retain_for_admission { return Err("shape is not a level-world lease".into()); }
        let count = *self.shape_references.get(index).ok_or("world shape references absent")?;
        if count == 0 { return Err("world shape already retired".into()); }
        if count > 1 { self.shape_references[index] -= 1; return Ok(()); }
        self.free_shapes.try_reserve(1).map_err(|error| error.to_string())?;
        self.shapes[index] = None;
        self.shape_references[index] = 0;
        self.free_shapes.push(index);
        Ok(())
    }

    /// Retain an actual standalone shape and each alias resource in the same
    /// geometry owner used by WorldStorage. One component lease owns one
    /// reference; the runtime body must be destroyed before it is released.
    pub(crate) fn register_standalone_shape(
        &mut self, lease: physworld::source::StandaloneShape,
        extra: Option<(physworld::source::StandaloneShape, [u16; 2])>,
    ) -> Result<usize, String> {
        use physworld::query_geometry::QueryGeometry;
        let mut aliases = Vec::new();
        aliases.try_reserve_exact(lease.wrappers.len()).map_err(|error| format!("shape alias identity key: {error}"))?;
        aliases.extend(lease.wrappers.iter().map(physworld::source::ShapeAlias::identity));
        let extra_key = extra.as_ref().map(|(shape, tags)| (std::sync::Arc::as_ptr(&shape.source) as usize,
            shape.wrappers.iter().map(physworld::source::ShapeAlias::identity).collect(), *tags));
        let key = StandaloneKey { source: std::sync::Arc::as_ptr(&lease.source) as usize, aliases, extra: extra_key };
        if let Some(&index) = self.standalone_keys.get(&key) {
            let asset = self.shapes.get_mut(index).and_then(Option::as_mut).ok_or("shared standalone shape slot absent")?;
            let existing=asset.standalone.as_mut().ok_or("shared standalone shape owner mismatch")?;
            if let Some(mass)=lease.mass.as_ref(){
                if existing.mass.as_ref().is_some_and(|old|!std::sync::Arc::ptr_eq(old,mass)){
                    return Err("shared standalone shape companion mass generation changed".into());
                }
                existing.mass=Some(mass.clone());
            }
            let count = self.shape_references.get_mut(index).ok_or("shared standalone reference count absent")?;
            *count = count.checked_add(1).ok_or("standalone shape reference count overflow")?;
            if trace_collision_sources() {
                eprintln!("COLLISION_SOURCE standalone_geometry slot={index} reused=true references={count}");
            }
            return Ok(index);
        }
        self.standalone_keys.try_reserve(1).map_err(|error| format!("standalone geometry identities: {error}"))?;
        let key = std::sync::Arc::new(key);
        let query_shapes = lease.query_shapes()?;
        let (query_shapes, _standalone_extra) = match extra {
            Some((extra, tags)) => (query_shapes.compound_pair(extra.query_shapes()?, tags)?, Some(Box::new(extra))),
            None => (query_shapes, None),
        };
        let root_shape = query_shapes.get(query_shapes.root())?;
        let header = root_shape.header;
        let is_mutable = match &root_shape.geometry {
            QueryGeometry::Compound(shape) => shape.is_mutable,
            QueryGeometry::Polytope(_) | QueryGeometry::Convex(_) | QueryGeometry::CompressedMesh(_) => false,
        };
        let identity = [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], [0., 0., 0., 1.]];
        let bounds = query_shapes.bounds(query_shapes.root(), identity)?;
        let path = lease.wrappers.first().map_or_else(|| lease.source_path.clone(), |resource| resource.path.clone());
        let source = lease.source.clone();
        let physics_property_present = _standalone_extra.is_none();
        let asset = TriggerShapeAsset { path, source, standalone: Some(Box::new(lease)), _standalone_extra, standalone_key: Some(key.clone()), root: 1,
            retain_for_admission: true, header, bounds, is_mutable,
            query_shapes: std::sync::Arc::new(query_shapes), cached_mass: None, signals: Default::default(), physics_property_present, physics_property: Default::default() };
        if let Some(index) = self.free_shapes.last().copied() {
            if self.shapes.get(index).is_none_or(Option::is_some) || self.shape_references.get(index) != Some(&0) {
                return Err("free shape slot is inconsistent".into());
            }
            self.free_shapes.pop();
            self.shapes[index] = Some(asset);
            self.shape_references[index] = 1;
            self.standalone_keys.insert(key, index);
            if trace_collision_sources() {
                eprintln!("COLLISION_SOURCE standalone_geometry slot={index} reused=false references=1 identities={} capacity={}", self.standalone_keys.len(), self.standalone_keys.capacity());
            }
            Ok(index)
        } else {
            self.shapes.try_reserve(1).map_err(|error| format!("standalone shape slot: {error}"))?;
            self.shape_references.try_reserve(1).map_err(|error| format!("standalone shape references: {error}"))?;
            let index = self.shapes.len();
            self.shapes.push(Some(asset));
            self.shape_references.push(1);
            self.standalone_keys.insert(key, index);
            if trace_collision_sources() {
                eprintln!("COLLISION_SOURCE standalone_geometry slot={index} reused=false references=1 identities={} capacity={}", self.standalone_keys.len(), self.standalone_keys.capacity());
            }
            Ok(index)
        }
    }

    /// Release after the actual body and all query users have retired.
    pub(crate) fn retire_standalone_shape(&mut self, index: usize) -> Result<(), String> {
        let asset = self.shapes.get(index).and_then(Option::as_ref).ok_or("standalone shape slot absent")?;
        if asset.standalone.is_none() { return Err("shape is not a standalone component lease".into()); }
        let count = *self.shape_references.get(index).ok_or("standalone reference count absent")?;
        if count == 0 { return Err("standalone shape reference count is zero".into()); }
        if count > 1 {
            self.shape_references[index] -= 1;
            if trace_collision_sources() {
                eprintln!("COLLISION_SOURCE standalone_retire slot={index} references={}", count - 1);
            }
            return Ok(());
        }
        let key = asset.standalone_key.as_ref().ok_or("standalone shape identity absent")?.clone();
        if self.standalone_keys.get(key.as_ref()) != Some(&index) { return Err("standalone shape identity map is inconsistent".into()); }
        self.free_shapes.try_reserve(1).map_err(|error| format!("standalone shape retirement: {error}"))?;
        self.standalone_keys.remove(key.as_ref());
        self.shapes[index] = None;
        self.shape_references[index] = 0;
        self.free_shapes.push(index);
        if trace_collision_sources() {
            eprintln!("COLLISION_SOURCE standalone_retire slot={index} references=0 identities={}", self.standalone_keys.len());
        }
        Ok(())
    }

    /// Release immutable serialized shapes after their typed query data and
    /// root mass result have been prepared. Mutable shapes keep their source.
    pub(crate) fn release_immutable_sources(&mut self) {
        let retain = std::env::var("DH2_DEBUG_RETAIN_TRIGGER_SOURCES").as_deref() == Ok("1");
        let trace = std::env::var("DH2_DEBUG_TRIGGER_SIGNATURES").as_deref() == Ok("1");
        let timing = std::env::var("DH2_DEBUG_TRIGGER_SOURCE_CPU").as_deref() == Ok("1");
        let start = if timing { crate::loading::thread_cycles() } else { None };
        let mut released = 0usize;
        let mut source_capacity = 0usize;
        let mut schema_capacity = 0usize;
        let mut mass_errors = 0usize;
        let mut held_source_capacity = 0usize;
        let mut held_schema_capacity = 0usize;
        let mut crc = flate2::Crc::new();
        for shape in self.shapes.iter_mut().flatten() {
            if shape.is_mutable || shape.retain_for_admission || shape.cached_mass.is_some() { continue; }
            if retain && !trace { continue; }
            let mass = shape.physics.shape_mass_properties(&shape.bytes, shape.root);
            if trace {
                mass_errors += usize::from(mass.is_err());
                crc.update(shape.path.as_bytes());
                crc.update(format!("{mass:?}{:?}", shape.query_shapes).as_bytes());
                crc.update(&[0]);
            }
            if retain { continue; }
            shape.cached_mass = Some(mass);
            if trace {
                source_capacity += shape.bytes.capacity();
                schema_capacity += physworld::source::schema_capacity(&shape.physics);
                if std::sync::Arc::strong_count(&shape.source) > 1 {
                    held_source_capacity += shape.bytes.capacity();
                    held_schema_capacity += physworld::source::schema_capacity(&shape.physics);
                }
            }
            shape.source = physworld::source::empty_source();
            released += 1;
        }
        if let (Some(start), Some(end)) = (start, if timing { crate::loading::thread_cycles() } else { None }) {
            eprintln!("MEMORY trigger source CPU: retained={retain} shapes={released} cycles={}", end.saturating_sub(start));
        }
        if trace {
            eprintln!("MEMORY trigger source retirement: retained={retain} shapes={released} source_capacity={source_capacity} schema_capacity={schema_capacity} signature={:08x} cache_field_capacity={} mass_errors={mass_errors} held_source_capacity={held_source_capacity} held_schema_capacity={held_schema_capacity} placement_slots={} shape_slots={} placement_slot_bytes={} shape_slot_bytes={} shape_reference_bytes={} free_index_bytes={}",
                crc.sum(), self.shapes.capacity() * std::mem::size_of::<Option<Result<Option<physworld::mass::PrincipalMassProperties>, String>>>(),
                self.placements.len(), self.shapes.len(),
                self.placements.capacity() * std::mem::size_of::<Option<TriggerPlacement>>() + self.shape_resources.capacity() * std::mem::size_of::<Option<usize>>(),
                self.shapes.capacity() * std::mem::size_of::<Option<TriggerShapeAsset>>(),
                self.shape_references.capacity() * std::mem::size_of::<usize>(),
                (self.free_shapes.capacity() + self.free_placements.capacity()) * std::mem::size_of::<usize>());
        }
    }

    /// Reads installed files without flattening compound instances, transforms,
    /// shape tags or tree order into movement triangles or volume planes.
    pub fn load(document: &Document, map: &str, catalog: &Catalog) -> Result<Self, String> {
        Self::load_with_sources(document, map, catalog, &mut physworld::source::ShapeSources::uncached(catalog))
    }

    pub(crate) fn load_with_sources(document: &Document, map: &str, catalog: &Catalog,
        sources: &mut physworld::source::ShapeSources) -> Result<Self, String> {
        let placements = trigger_placements(document, map, |name| {
            catalog
                .read(name)
                .map(|bytes| bytes.map(|b| declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        })?;
        Self::from_placements_with_sources(placements, catalog, sources)
    }

    /// Acquire shapes for the actual admitted placements, preserving the
    /// shared source catalog and one identity per shape resource.
    pub(crate) fn from_placements_with_sources(placements: Vec<TriggerPlacement>, catalog: &Catalog,
        sources: &mut physworld::source::ShapeSources) -> Result<Self, String> {
        crate::gameplay::physics_world::thread_policy::enter();
        let mut assets = Self { placements: Vec::new(), shape_resources: Vec::new(), shapes: Vec::new(),
            free_placements: Vec::new(), free_shapes: Vec::new(), shape_references: Vec::new(), standalone_keys: HashMap::new() };
        assets.append_placements_with_sources(placements, catalog, sources)?;
        Ok(assets)
    }

    /// Prepare another admitted batch against the same retained shape owner.
    /// No placement is published until every shape in this batch is acquired.
    pub(crate) fn append_placements_with_sources(&mut self, placements: Vec<TriggerPlacement>, catalog: &Catalog,
        sources: &mut physworld::source::ShapeSources) -> Result<Vec<usize>, String> {
        if placements.is_empty() { return Ok(Vec::new()); }
        let mut shapes = Vec::new();
        let mut indices: HashMap<_, _> = self.shapes.iter().enumerate().filter_map(|(index, shape)| {
            let shape = shape.as_ref()?;
            (!shape.retain_for_admission && shape.root == 1).then(|| (shape.path.clone(), index))
        }).collect();
        let mut shape_resources = Vec::with_capacity(placements.len());
        for placement in &placements {
            let path = placement.shape.asset_path();
            let index = match indices.get(&path) {
                Some(&index) => index,
                None => {
                    let source = sources.acquire(catalog, &path, physworld::source::ShapeUse::Physics)
                            .map_err(|e| match e { physworld::source::ShapeSourceError::Read(e) => e,
                                physworld::source::ShapeSourceError::Parse(e) => format!("{path}: {e}") })?
                            .ok_or_else(|| {
                                format!(
                                    "{}: missing trigger shape {path}",
                                    placement.binding.entity_id
                                )
                            })?;
                    let bytes = &source.bytes;
                    let physics = &source.physics;
                    let root = physics
                        .items
                        .get(1)
                        .ok_or_else(|| format!("{path}: no root shape item"))?;
                    let kind = &physics.types[root.type_index].name;
                    if !kind.ends_with("Shape") {
                        return Err(format!("{path}: root item is {kind}, not a shape"));
                    }
                    let header = physics
                        .shape_header(&bytes, 1)
                        .map_err(|e| format!("{path}: {e}"))?;
                    let bounds = physics
                        .compound_bounds(&bytes, 1)
                        .map_err(|e| format!("{path}: {e}"))?;
                    let is_mutable = physics
                        .compound_is_mutable(&bytes, 1)
                        .map_err(|e| format!("{path}: {e}"))?;
                    let index = self.free_shapes.iter().rev().nth(shapes.len()).copied()
                        .unwrap_or_else(|| self.shapes.len() + shapes.len().saturating_sub(self.free_shapes.len()));
                    let query_shapes = physics
                        .query_shapes(&bytes, 1)
                        .map_err(|e| format!("{path}: {e}"))?;
                    shapes.push((index, TriggerShapeAsset {
                        path: path.clone(),
                        source,
                        standalone: None,
                        _standalone_extra: None,
                        standalone_key: None,
                        root: 1,
                        retain_for_admission: false,
                        header,
                        bounds,
                        is_mutable,
                        query_shapes: std::sync::Arc::new(query_shapes),
                        cached_mass: None,
                        signals: Default::default(), physics_property_present: false, physics_property: Default::default(),
                    }));
                    indices.insert(path, index);
                    index
                }
            };
            shape_resources.push(index);
        }
        let new_shapes = shapes.len().saturating_sub(self.free_shapes.len());
        let new_placements = placements.len().saturating_sub(self.free_placements.len());
        for &index in self.free_shapes.iter().rev().take(shapes.len()) {
            if self.shapes.get(index).is_none_or(Option::is_some)
                || self.shape_references.get(index) != Some(&0) {
                return Err("invalid free trigger shape slot".into());
            }
        }
        for &index in self.free_placements.iter().rev().take(placements.len()) {
            if self.placements.get(index).is_none_or(Option::is_some)
                || self.shape_resources.get(index).is_none_or(Option::is_some) {
                return Err("invalid free trigger placement slot".into());
            }
        }
        let mut increments = HashMap::<usize, usize>::new();
        for &index in &shape_resources {
            let value = increments.entry(index).or_default();
            *value = value.checked_add(1).ok_or("trigger shape reference overflow")?;
        }
        for (&index, &increment) in &increments {
            self.shape_references.get(index).copied().unwrap_or(0).checked_add(increment)
                .ok_or("trigger shape reference overflow")?;
        }
        self.shapes.try_reserve(new_shapes).map_err(|error| format!("trigger shape slots: {error}"))?;
        self.shape_references.try_reserve(new_shapes).map_err(|error| format!("trigger shape counts: {error}"))?;
        self.placements.try_reserve(new_placements).map_err(|error| format!("trigger placement slots: {error}"))?;
        self.shape_resources.try_reserve(new_placements).map_err(|error| format!("trigger resource slots: {error}"))?;
        let mut result = Vec::new();
        result.try_reserve_exact(placements.len()).map_err(|error| format!("trigger placement indices: {error}"))?;
        for (index, shape) in shapes {
            if index < self.shapes.len() {
                self.free_shapes.pop();
                self.shapes[index] = Some(shape);
                self.shape_references[index] = 0;
            } else {
                self.shapes.push(Some(shape));
                self.shape_references.push(0);
            }
        }
        for (placement, shape) in placements.into_iter().zip(shape_resources) {
            self.shape_references[shape] += 1;
            let index = if let Some(index) = self.free_placements.pop() {
                self.placements[index] = Some(placement);
                self.shape_resources[index] = Some(shape);
                index
            } else {
                let index = self.placements.len();
                self.placements.push(Some(placement));
                self.shape_resources.push(Some(shape));
                index
            };
            result.push(index);
        }
        Ok(result)
    }

    /// Called only after the component's body and shape subscriptions retire.
    /// Other live indices remain stable, and the final placement lease drops
    /// its shape resource immediately.
    pub(crate) fn retire_placement(&mut self, index: usize) -> Result<(), String> {
        let shape = self.shape_resources.get(index).and_then(|value| *value).ok_or("trigger resource slot absent")?;
        if self.placements.get(index).and_then(Option::as_ref).is_none() { return Err("trigger placement slot absent".into()); }
        let count = *self.shape_references.get(shape).ok_or("trigger shape reference count absent")?;
        let asset = self.shapes.get(shape).and_then(Option::as_ref).ok_or("trigger shape resource absent")?;
        if count == 0 { return Err("trigger shape reference count is zero".into()); }
        let retire_shape = count == 1 && !asset.retain_for_admission;
        self.free_placements.try_reserve(1).map_err(|error| format!("trigger placement retirement: {error}"))?;
        if retire_shape { self.free_shapes.try_reserve(1).map_err(|error| format!("trigger shape retirement: {error}"))?; }
        self.placements[index] = None;
        self.shape_resources[index] = None;
        self.free_placements.push(index);
        self.shape_references[shape] -= 1;
        if retire_shape { self.shapes[shape] = None; self.free_shapes.push(shape); }
        if std::env::var("DH2_DEBUG_TRIGGER_SIGNATURES").as_deref() == Ok("1") {
            eprintln!("MEMORY trigger placement retired: placement={index} shape={shape} shape_retired={retire_shape} remaining={} free_placements={} free_shapes={}",
                self.shape_references[shape], self.free_placements.len(), self.free_shapes.len());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::{
        motion::Motion,
        space_cells::SpaceCells,
        trigger_body::{prepare_trigger_body, TriggerBodyInput},
    };

    #[test]
    #[ignore = "requires installed level/trigger archives"]
    fn installed_trigger_motion_preparation() {
        let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        let cells = SpaceCells::dynamic(16).unwrap();
        let mut counts = [0usize; 5];
        let mut mutable_shapes = 0usize;
        for path in catalog.paths_with_suffix(".bwm") {
            let level = crate::assets::level::LevelResources::resolve(&catalog, &path).unwrap();
            let bytes = catalog.read(&level.entities).unwrap().unwrap();
            let document = crate::assets::entities::parse(&declaration_text(&bytes)).unwrap();
            let map = format!(
                "{}.map",
                level
                    .entities
                    .strip_prefix("maps/")
                    .unwrap()
                    .strip_suffix(".entities")
                    .unwrap()
            );
            let assets = TriggerAssets::load(&document, &map, &catalog).unwrap();
            for (placement, &resource) in assets.placements.iter().zip(&assets.shape_resources) {
                let placement = placement.as_ref().unwrap();
                let resource = resource.unwrap();
                // Material/user-data handles do not enter motion preparation.
                // Supply neutral test handles; world ownership remains separate.
                let body = prepare_trigger_body(TriggerBodyInput {
                    shape: 1,
                    position: placement.position,
                    basis: placement.shape.basis,
                    material_id: 0,
                    user_data: 0,
                    water_component: placement.binding.water_component,
                    wind_component: placement.binding.wind_component,
                    collision: placement.binding.collision,
                });
                let shape = assets.shapes[resource].as_ref().unwrap();
                let info = shape
                    .prepare_motion(&body, None)
                    .unwrap_or_else(|e| panic!("{} {}: {e}", path, placement.binding.entity_id));
                assert!(
                    info.pose
                        .center
                        .into_iter()
                        .chain(info.pose.orientation)
                        .all(f32::is_finite),
                    "{} {}",
                    path,
                    placement.binding.entity_id
                );
                let motion = Motion::initialize(&info, &cells).unwrap();
                assert!(motion.cell < 16);
                if body.motion_type == 1 {
                    assert_eq!(info.inverse_mass, 0.);
                    assert_eq!(info.inverse_inertia, [0.; 4]);
                    counts[2] += 1;
                }
                counts[3] += usize::from(placement.binding.water_component);
                counts[4] += usize::from(
                    shape
                        .physics
                        .shape_mass_properties(&shape.bytes, 1)
                        .unwrap()
                        .is_some(),
                );
                counts[1] += 1;
                mutable_shapes += usize::from(shape.is_mutable);
            }
            counts[0] += 1;
        }
        assert_eq!(&counts[..3], &[36, 5576, 23]);
        println!("Installed worlds/triggers/keyframed/water/cached-mass={counts:?}");
        println!("Mutable trigger shapes={mutable_shapes}");
    }
}
