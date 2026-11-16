//! Navmesh resource admission metadata.083ab0 reads the0x4e4d000f
//! container;083040 reads each actual layer record and its three Havok
//! streams.082ec0/1b1510 use hkaiNavMesh.userData's packed world/instance
//! identity. This is not a second navigation geometry or pathfinding owner.
use super::{le32, parse_embedded_havok, section, slice};
use crate::archive::Catalog;
use std::{ops::Range, sync::{Arc, Mutex}};
mod dsm;
pub(crate) use dsm::{Cell, Graph};

#[derive(Debug)]
pub(crate) struct Layer {
    pub(crate) settings: Box<str>,
    pub(crate) world: u16,
    pub(crate) instance: u16,
    pub(crate) mesh: Range<usize>,
    pub(crate) cluster_graph: Range<usize>,
    pub(crate) user_edges: Range<usize>,
    pub(crate) jump_info: Range<usize>,
    pub(crate) dsm_info: Range<usize>,
}
#[derive(Debug)]
pub(crate) struct Metadata {
    pub(crate) layers: Vec<Layer>, bytes: Vec<u8>,
    graphs: Mutex<std::collections::HashMap<(usize, usize), std::sync::Weak<Graph>>>,
}

#[derive(Default)]
pub(crate) struct Resources(Mutex<crate::assets::resource_registry::Registry<Metadata>>);

pub(crate) fn register_preload(catalog: &Catalog, path: &str, marker: u8) -> Result<(), String> {
    catalog.navmesh_resources.get_or_init(Default::default).0.lock().map_err(|_| "navmesh registry is poisoned")?
        .register(path, marker, false);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.navmesh_resources.get() {
        owner.0.lock().map_err(|_| "navmesh registry is poisoned")?.promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.navmesh_resources.get() {
        let mut owner = owner.0.lock().unwrap_or_else(|error| error.into_inner());
        owner.release(catalog.identity(), "Navmesh", marker);
        owner.report_with(catalog.identity(), "Navmesh", "scope-released", Metadata::storage_bytes, String::capacity);
    }
}
pub(crate) fn report_retirement(catalog: &Catalog) {
    if let Some(owner) = catalog.navmesh_resources.get() {
        owner.0.lock().unwrap_or_else(|error| error.into_inner())
            .report_with(catalog.identity(), "Navmesh", "catalog-retiring", Metadata::storage_bytes, String::capacity);
    }
}

///8aae30 constructs maps/<actual map name>.Navmesh before1af730. The
///existing entities companion names that same actual map, not its BWM file.
pub(crate) fn companion(level: &crate::assets::level::LevelResources) -> Result<String, String> {
    let map = level.entities.strip_suffix(".entities").ok_or("level has no entities companion identity")?;
    Ok(format!("generated/navmesh/{map}.bnavmesh"))
}
pub(crate) fn navigation_admission(catalog: &Catalog, level: &crate::assets::level::LevelResources) -> Result<usize, String> {
    let path = companion(level)?;
    //1af730 returnsfalse for an absent actual resource;8aae30 ignores that
    //return. The realworld then owns no admitted instances.
    let Some(metadata) = Metadata::acquire_optional(catalog, &path)? else { return Ok(0) };
    let mut admission = Admission::default();
    admission.admit(&metadata)?;
    Ok(admission.count(0))
}

struct Reader<'a> { bytes: &'a [u8], at: usize }
impl<'a> Reader<'a> {
    fn text(&mut self) -> Result<&'a str, String> {
        let length = le32(self.bytes, self.at)? as usize;
        self.at = self.at.checked_add(4).ok_or("navmesh string offset overflow")?;
        let bytes = slice(self.bytes, self.at, length)?;
        self.at = self.at.checked_add(length).ok_or("navmesh string length overflow")?;
        std::str::from_utf8(bytes).map_err(|error| format!("navmesh text: {error}"))
    }
    fn tag(&mut self) -> Result<&'a [u8], String> {
        let tag = section(self.bytes, self.at)?;
        if tag.tag != *b"TAG0" { return Err("navmesh section is not a Havok TAG0 stream".into()); }
        let bytes = &self.bytes[self.at..tag.end];
        self.at = tag.end;
        Ok(bytes)
    }
}
impl Metadata {
    fn storage_bytes(&self) -> usize {
        self.bytes.capacity() + self.layers.capacity() * std::mem::size_of::<Layer>()
            + self.layers.iter().map(|layer| layer.settings.len()).sum::<usize>()
    }
    pub(crate) fn acquire(catalog: &Catalog, path: &str) -> Result<Arc<Self>, String> {
        Self::acquire_optional(catalog, path)?.ok_or_else(|| format!("selected navmesh resource {path} is absent"))
    }
    pub(crate) fn acquire_optional(catalog: &Catalog, path: &str) -> Result<Option<Arc<Self>>, String> {
        if !catalog.contains(path) { return Ok(None); }
        let owner = catalog.navmesh_resources.get_or_init(Default::default);
        let shared = !catalog.has_overlay_file(path);
        owner.0.lock().map_err(|_| "navmesh registry is poisoned")?.acquire_with_sharing(path, 0, shared, |_| {
            let bytes = catalog.read(path).map_err(|error| error.to_string())?
                .ok_or_else(|| format!("selected navmesh resource {path} is absent"))?;
            Self::from_bytes(bytes).map(Arc::new).map_err(|error| format!("{path}: {error}"))
        }).map(Some)
    }
    pub(crate) fn read(bytes: &[u8]) -> Result<Self, String> {
        Self::from_bytes(bytes.to_vec())
    }
    fn from_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        let source = &*bytes;
        if le32(source, 0)? != 0x4e4d000f { return Err("unsupported navmesh resource version".into()); }
        let count = le32(source, 4)? as usize;
        if count > bytes.len() / 8 { return Err("navmesh layer table is truncated".into()); }
        let mut reader = Reader { bytes: source, at: 8 };
        let mut layers = Vec::with_capacity(count);
        for _ in 0..count {
            let settings = reader.text()?.into();
            let mesh_start = reader.at;
            let mesh = reader.tag()?;
            let mesh_range = mesh_start..reader.at;
            let schema = parse_embedded_havok(mesh)?;
            let root = schema.items.get(1).ok_or("navmesh stream has no root item")?;
            let typ = schema.types.get(root.type_index).ok_or("navmesh root type is absent")?;
            if typ.name != "hkaiNavMesh" || root.count != 1 {
                return Err(format!("navmesh stream root is {} instead of one hkaiNavMesh", typ.name));
            }
            let data = schema.item_at(mesh, 1, 0)?;
            let offset = schema.field_offset(root.type_index, "userData")?;
            let identity = le32(data, offset)?;
            let start = reader.at;
            reader.tag()?; // hkaiClusterGraph,081070
            let cluster_graph = start..reader.at;
            let start = reader.at + 4;
            reader.text()?; // jumpEdgeInfoList
            let jump_info = start..reader.at;
            let start = reader.at;
            reader.tag()?; // hkaiUserEdgePairArray,0811b0
            let user_edges = start..reader.at;
            let start = reader.at + 4;
            reader.text()?;
            let dsm_info = start..reader.at;
            layers.push(Layer { settings, world: (identity >> 16) as u16, instance: identity as u16,
                mesh: mesh_range, cluster_graph, user_edges, jump_info, dsm_info });
        }
        if reader.at != bytes.len() { return Err("unexpected trailing navmesh resource bytes".into()); }
        Ok(Self { layers, bytes, graphs: Default::default() })
    }
    pub(crate) fn bytes(&self, range: &Range<usize>) -> &[u8] { &self.bytes[range.clone()] }
}

/// One map admission, in the actual mounted resource order.1ab0b0 requires
/// each instance ordinal to equal its world's current admitted count.
#[derive(Default)]
pub(crate) struct Admission { worlds: Vec<usize> }
impl Admission {
    pub(crate) fn admit(&mut self, metadata: &Metadata) -> Result<(), String> {
        for layer in &metadata.layers {
            let world = usize::from(layer.world);
            if self.worlds.len() <= world { self.worlds.resize(world + 1, 0); }
            let count = &mut self.worlds[world];
            if *count != usize::from(layer.instance) {
                return Err(format!("navmesh layer {} world{} instance{} does not follow admitted count{}",
                    layer.settings, world, layer.instance, count));
            }
            *count += 1;
        }
        Ok(())
    }
    pub(crate) fn count(&self, world: u16) -> usize {
        self.worlds.get(usize::from(world)).copied().unwrap_or(0)
    }
}
