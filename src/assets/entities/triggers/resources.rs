//! Owned trigger shape files. Item references remain local to their HKX;
//! resource indices here are not runtime body IDs or Havok item references.
use super::{trigger_placements, Document, TriggerPlacement};
use crate::assets::{
    archive::Catalog,
    entities::declaration_text,
    physworld::{self, PhysWorld},
};
use std::collections::HashMap;

pub struct TriggerShapeAsset {
    pub path: String,
    // Source is retired only after cached_mass has been filled. Runtime users
    // obtain mass through the methods below and geometry through query_shapes.
    pub(crate) bytes: Vec<u8>,
    pub(crate) physics: PhysWorld,
    pub header: physworld::shape::ShapeHeader,
    pub bounds: [[f32; 4]; 2],
    pub is_mutable: bool,
    pub query_shapes: std::sync::Arc<physworld::query_geometry::QueryShapes>,
    cached_mass: Option<Result<Option<physworld::mass::PrincipalMassProperties>, String>>,
    pub(crate) signals: crate::gameplay::shape_registry::ShapeSignals,
}

impl crate::gameplay::shape_registry::ShapeSignalStore for TriggerAssets {
    fn signals_mut(&mut self,shape:usize)->Result<&mut crate::gameplay::shape_registry::ShapeSignals,String>{
        let shape=self.shapes.get_mut(shape).ok_or("trigger shape signal resource is missing")?;
        if !shape.is_mutable {return Err("immutable trigger shape has no mutation signals".into());}
        Ok(&mut shape.signals)
    }
}

impl crate::gameplay::physics_world::body_creation::BodyCreationGeometry for TriggerAssets {
    fn creation_assets(&self,info:&physworld::body::BodyInfo)
        ->Result<crate::gameplay::body_storage::BodyCreationAssets<'_>,String>{
        let shape=self.shapes.get(info.shape).ok_or("trigger creation shape is missing")?;
        //779530 leaves these references null in its actual Cinfo recipe. Do not
        // silently reinterpret an unrelated body's unresolved references here.
        if info.name!=0 || info.mass_distribution!=0 {
            return Err("trigger Cinfo has unresolved name or mass-distribution references".into());
        }
        Ok(crate::gameplay::body_storage::BodyCreationAssets { shape:shape.header,
            wrapped_shapes:None,name:None,mass_distribution:None })
    }
    fn header(&self,shape:usize)->Result<physworld::shape::ShapeHeader,String>{
        Ok(self.shapes.get(shape).ok_or("trigger shape header is missing")?.header)
    }
    fn is_mutable(&self,shape:usize)->Result<bool,String>{
        Ok(self.shapes.get(shape).ok_or("trigger shape mutability resource is missing")?.is_mutable)
    }
    fn rotation_limit_measure(&self,shape:usize)->Result<f32,String>{
        self.shapes.get(shape).ok_or("trigger rotation-limit shape is missing")?;
        //Both actual compound vtables point +68 toc96770, an INT3 virtual.
        Err("compound shape does not implement rotation-limit measurement".into())
    }
    fn cached_mass(&self,shape:usize)->Result<Option<physworld::mass::PrincipalMassProperties>,String>{
        let shape=self.shapes.get(shape).ok_or("trigger mass shape is missing")?;
        shape.root_mass()
    }
    fn calculate_mass(&self,shape:usize,config:[u32;3])->Result<physworld::mass::PrincipalMassProperties,String>{
        let shape=self.shapes.get(shape).ok_or("trigger calculated-mass shape is missing")?;
        crate::gameplay::motion_info::calculate_compound_mass(config,shape.bounds)
    }
}

impl TriggerShapeAsset {
    fn root_mass(&self) -> Result<Option<physworld::mass::PrincipalMassProperties>, String> {
        self.cached_mass.clone().unwrap_or_else(|| self.physics.shape_mass_properties(&self.bytes, 1))
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
    pub placements: Vec<TriggerPlacement>,
    /// Parallel to placements, selecting the shape asset for each trigger.
    pub shape_resources: Vec<usize>,
    pub shapes: Vec<TriggerShapeAsset>,
}

impl crate::gameplay::physics_world::PublicationGeometry for TriggerAssets {
    fn is_null(&mut self, shape: usize) -> Result<bool, String> {
        // Every retained asset has a real root. Zero is a valid resource index;
        // an absent resource is an error, never the source's null pointer state.
        self.shapes.get(shape).ok_or_else(|| format!("missing trigger shape resource {shape}"))?;
        Ok(false)
    }

    fn bounds(
        &mut self, shape: usize, frame: crate::gameplay::shape_transform::ShapeMatrix,
    ) -> Result<[[f32; 4]; 2], String> {
        let shape = self.shapes.get(shape).ok_or_else(|| format!("missing trigger shape resource {shape}"))?;
        shape.query_shapes.bounds(shape.query_shapes.root(), frame)
    }
}

impl TriggerAssets {
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
        let mut crc = flate2::Crc::new();
        for shape in &mut self.shapes {
            if shape.is_mutable || shape.cached_mass.is_some() { continue; }
            if retain && !trace { continue; }
            let mass = shape.physics.shape_mass_properties(&shape.bytes, 1);
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
                let physics = &shape.physics;
                schema_capacity += physics.sdk_version.capacity()
                    + physics.materials.capacity() * std::mem::size_of::<String>()
                    + physics.materials.iter().map(String::capacity).sum::<usize>()
                    + physics.material_map.capacity() * std::mem::size_of::<u32>()
                    + physics.items.capacity() * std::mem::size_of::<physworld::TagItem>()
                    + physics.types.capacity() * std::mem::size_of::<physworld::TagType>()
                    + physics.types.iter().map(|ty| ty.name.capacity()
                        + ty.fields.capacity() * std::mem::size_of::<physworld::TagField>()
                        + ty.fields.iter().map(|field| field.name.capacity()).sum::<usize>()).sum::<usize>();
            }
            shape.bytes = Vec::new();
            shape.physics = PhysWorld { materials: Vec::new(), material_map: Vec::new(),
                sdk_version: String::new(), types: Vec::new(), items: Vec::new(), data_start: 0, data_size: 0 };
            released += 1;
        }
        if let (Some(start), Some(end)) = (start, if timing { crate::loading::thread_cycles() } else { None }) {
            eprintln!("MEMORY trigger source CPU: retained={retain} shapes={released} cycles={}", end.saturating_sub(start));
        }
        if trace {
            eprintln!("MEMORY trigger source retirement: retained={retain} shapes={released} source_capacity={source_capacity} schema_capacity={schema_capacity} signature={:08x} cache_field_capacity={} mass_errors={mass_errors}", crc.sum(), self.shapes.capacity() * std::mem::size_of::<Option<Result<Option<physworld::mass::PrincipalMassProperties>, String>>>());
        }
    }

    /// Reads installed files without flattening compound instances, transforms,
    /// shape tags or tree order into movement triangles or volume planes.
    pub fn load(document: &Document, map: &str, catalog: &Catalog) -> Result<Self, String> {
        let placements = trigger_placements(document, map, |name| {
            catalog
                .read(name)
                .map(|bytes| bytes.map(|b| declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        })?;
        let mut shapes = Vec::new();
        let mut indices = HashMap::new();
        let mut shape_resources = Vec::with_capacity(placements.len());
        for placement in &placements {
            let path = placement.shape.asset_path();
            let index = match indices.get(&path) {
                Some(&index) => index,
                None => {
                    let bytes =
                        catalog
                            .read(&path)
                            .map_err(|e| e.to_string())?
                            .ok_or_else(|| {
                                format!(
                                    "{}: missing trigger shape {path}",
                                    placement.binding.entity_id
                                )
                            })?;
                    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                    if *TRACE.get_or_init(|| std::env::var("DH2_DEBUG_TRIGGER_SOURCES").as_deref() == Ok("1")) {
                        let mut crc = flate2::Crc::new(); crc.update(&bytes);
                        eprintln!("MEMORY trigger source acquisition: owner=physics path={path} bytes={} signature={:08x}", bytes.len(), crc.sum());
                    }
                    let physics =
                        physworld::parse_hkx(&bytes).map_err(|e| format!("{path}: {e}"))?;
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
                    let index = shapes.len();
                    let query_shapes = physics
                        .query_shapes(&bytes, 1)
                        .map_err(|e| format!("{path}: {e}"))?;
                    shapes.push(TriggerShapeAsset {
                        path: path.clone(),
                        bytes,
                        physics,
                        header,
                        bounds,
                        is_mutable,
                        query_shapes: std::sync::Arc::new(query_shapes),
                        cached_mass: None,
                        signals: Default::default(),
                    });
                    indices.insert(path, index);
                    index
                }
            };
            shape_resources.push(index);
        }
        Ok(Self {
            placements,
            shape_resources,
            shapes,
        })
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
                let shape = &assets.shapes[resource];
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
