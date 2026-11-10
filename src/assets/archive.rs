//! Read-only access to the installed game's resource indices and archives.
pub mod index;
mod containers;
use flate2::read::ZlibDecoder;
use std::error::Error;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Copy)]
struct Entry {
    group: u8,
    /// Primary header tables containing this identity, before payload overrides.
    header_groups: u8,
    offset: u64,
    size: u32,
    stored_size: u32,
    slot: u16,
    /// Index into `Catalog::kinds`.
    kind: u16,
    flags: u16,
}

impl Entry {
    fn is_streamed(&self) -> bool {
        self.flags & 0x20 != 0
    }
}

/// Sorted strings packed into one buffer. The installed index lists about a
/// million resource paths; as separate `String`s their headers and
/// allocations cost more than their text.
#[derive(Default)]
struct Names {
    text: String,
    ends: Vec<u32>,
}

impl Names {
    fn new<S: AsRef<str>>(names: impl IntoIterator<Item = S>) -> Self {
        let mut packed = Self::default();
        for name in names {
            packed.text.push_str(name.as_ref());
            packed.ends.push(packed.text.len() as u32);
        }
        packed.text.shrink_to_fit();
        packed.ends.shrink_to_fit();
        packed
    }

    fn get(&self, index: usize) -> &str {
        let start = index.checked_sub(1).map_or(0, |i| self.ends[i] as usize);
        &self.text[start..self.ends[index] as usize]
    }

    fn len(&self) -> usize {
        self.ends.len()
    }

    fn iter(&self) -> impl Iterator<Item = &str> {
        (0..self.len()).map(|i| self.get(i))
    }

    fn search(&self, key: &str) -> std::result::Result<usize, usize> {
        let index = self.partition_point(|name| name < key);
        if index < self.len() && self.get(index) == key { Ok(index) } else { Err(index) }
    }

    fn partition_point(&self, before: impl Fn(&str) -> bool) -> usize {
        let (mut low, mut high) = (0, self.len());
        while low < high {
            let middle = (low + high) / 2;
            if before(self.get(middle)) { low = middle + 1 } else { high = middle }
        }
        low
    }
}

pub struct Catalog {
    /// Distinguish independently opened catalogs for session metadata caches.
    identity: u64,
    /// One session owner for live AnimSet declaration identities.
    pub(crate) animset_declarations: std::sync::Mutex<crate::assets::animation_rig::animset::Registry>,
    /// Shared registered MD6 declaration headers and immutable metadata.
    pub(crate) md6_declarations: std::sync::Mutex<crate::assets::animation_rig::declaration::Registry>,
    pub(crate) md6_loading: std::sync::Mutex<()>,
    /// One resource-list identity owner for this catalog's animation clips.
    pub(crate) animation_resources: std::sync::Mutex<Option<crate::assets::edge_animation::residency::Registry>>,
    /// Weak live skeleton identities for this archive namespace.
    pub(crate) skeleton_resources: std::sync::OnceLock<std::sync::Arc<crate::assets::edge_skeleton::Registry>>,
    /// One lazy class owner shared by particle and material parameter blocks.
    pub(crate) parameter_blocks: std::sync::OnceLock<std::sync::Arc<std::sync::Mutex<crate::assets::parm_block::Registry>>>,
    pub(crate) component_types: std::sync::Mutex<Option<std::sync::Arc<crate::gameplay::component_state::types::ComponentTypes>>>,
    pub(crate) material_constants: std::sync::Mutex<Option<std::sync::Arc<crate::assets::physics_material::MaterialConstants>>>,
    pub(crate) fade_resources: std::sync::OnceLock<std::sync::Mutex<crate::assets::resource_registry::Registry<crate::assets::fx::fade::Fade>>>,
    pub(crate) physics_material_resources: std::sync::OnceLock<std::sync::Mutex<crate::assets::resource_registry::Registry<crate::assets::entities::physics_material::resources::Definition>>>,
    pub(crate) physics_system_resources: std::sync::OnceLock<crate::assets::physworld::physics_system::Resources>,
    pub(crate) mass_resources: std::sync::OnceLock<std::sync::Mutex<crate::assets::resource_registry::Registry<Option<crate::assets::physworld::mass::PrincipalMassProperties>>>>,
    pub(crate) common_models: std::sync::OnceLock<std::result::Result<crate::assets::bmodel::CommonModels, String>>,
    pub(crate) difficulty_definitions: std::sync::OnceLock<std::sync::Mutex<crate::assets::resource_registry::Registry<crate::assets::difficulty::Definition>>>,
    pub(crate) difficulty_presets: std::sync::OnceLock<std::sync::Mutex<crate::assets::resource_registry::Registry<crate::assets::difficulty::Preset>>>,
    pub(crate) sound_rtpcs: std::sync::OnceLock<std::sync::Mutex<crate::assets::resource_registry::Registry<crate::assets::wwise::RtpcDeclaration>>>,
    pub(crate) sound_auxiliary_buses: std::sync::OnceLock<std::sync::Mutex<crate::assets::resource_registry::Registry<u32>>>,
    pub(crate) sound_events: std::sync::OnceLock<std::sync::Mutex<crate::assets::resource_registry::Registry<crate::assets::wwise::SoundEventDeclaration>>>,
    pub(crate) kiscule_resources: std::sync::OnceLock<std::sync::Mutex<crate::assets::resource_registry::Registry<crate::assets::entities::effects::FxGraph>>>,
    pub(crate) particle_resources: std::sync::OnceLock<crate::assets::fx::particle_resources::Owner>,
    pub(crate) render_parameters: std::sync::OnceLock<crate::assets::render_parameter::Owner>,
    pub(crate) shader_indices: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<
        crate::assets::shader_cache::ShaderCacheBackend, std::sync::Arc<crate::assets::shader_cache::ShaderIndex>>>>,
    /// Inventory/history leases can outlive a map's item acquisition table.
    pub(crate) pickable_sources: std::sync::OnceLock<std::sync::Arc<std::sync::Mutex<crate::assets::inventory_item::PickableSources>>>,
    /// Prepared session resources resolved from installed declarations.
    pub(crate) camera_blur: std::sync::Mutex<Option<std::sync::Arc<crate::assets::camera_blur::BlurResources>>>,
    pub(crate) contact_database: std::sync::Mutex<Option<std::sync::Arc<crate::assets::entities::contacts::ContactDatabase>>>,
    /// Installed init/common declaration texts for the current session language.
    pub(crate) common_declaration_texts: std::sync::Mutex<Option<crate::assets::declaration_texts::SharedTexts>>,
    /// The index kinds (`image`, `material`, ...) of the entries.
    kinds: Vec<String>,
    base: PathBuf,
    /// Sorted resource paths, and the entry of each.
    paths: Names,
    entries: Vec<Entry>,
    // Loose resource files of the rewrite's enabled mods by normalized
    // resource path, sorted; they win over the archives.
    overlay: Vec<(String, PathBuf)>,
    // Image source names are authoritative index aliases, including imageprog
    // expressions and option suffixes. Values index the final override table.
    image_sources: (Names, Vec<u32>),
    image_programs: (Names, Vec<u32>),
    containers: containers::Containers,
    /// 068c30/0658b0 primary header order, distinct from data archive slots.
    header_order: std::sync::OnceLock<std::result::Result<Box<[u8]>, String>>,
    pub(crate) manifests: std::sync::OnceLock<crate::assets::map_resources::ManifestCache>,
    pub(crate) map_info: std::sync::OnceLock<crate::gameplay::map_info::MapInfoCache>,
    pub(crate) language_table: std::sync::Mutex<Option<crate::assets::lang::LoadedTable>>,
    pub(crate) preload_bootstrap: std::sync::OnceLock<std::result::Result<usize, String>>,
    pub(crate) preload_map: std::sync::Mutex<Option<Box<str>>>,
}

impl Drop for Catalog {
    fn drop(&mut self) {
        if let Some(owner) = self.pickable_sources.get() {
            owner.lock().unwrap_or_else(|error| error.into_inner()).release_session(self);
        }
        if let Some(owner) = self.parameter_blocks.get() {
            owner.lock().unwrap_or_else(|error| error.into_inner()).release_session();
        }
        self.animset_declarations.get_mut().unwrap_or_else(|error| error.into_inner()).release_session();
        if std::env::var_os("DH2_DEBUG_ANIMSET_LIFETIMES").is_some_and(|value| value == "1") {
            eprintln!("ANIMSET_SESSION_RELEASE catalog={}", self.identity);
        }
        crate::assets::fx::fade::report_retirement(self);
        self.fade_resources.take();
        crate::assets::entities::physics_material::resources::report_retirement(self);
        self.physics_material_resources.take();
        crate::assets::physworld::physics_system::report_retirement(self);
        self.physics_system_resources.take();
        crate::assets::physworld::mass::resources::report_retirement(self);
        crate::assets::wwise::event_resources::report_retirement(self);
        self.mass_resources.take();
        crate::assets::wwise::auxiliary_resources::report_retirement(self);
        crate::assets::wwise::rtpc_resources::report_retirement(self);
        crate::assets::difficulty::report_retirement(self);
        self.difficulty_definitions.take();
        self.difficulty_presets.take();
        self.sound_rtpcs.take();
        self.sound_auxiliary_buses.take();
        self.sound_events.take();
        self.kiscule_resources.take();
        self.particle_resources.take();
        crate::assets::render_parameter::report(self, "catalog-retiring");
        let parameters = usize::from(self.render_parameters.take().is_some_and(|mut owner| {
            owner.shader_metadata.get_mut().unwrap_or_else(|error| error.into_inner()).take().is_some()
        }));
        self.shader_indices.take();
        // Host read accounting belongs to this immutable archive namespace;
        // releasing it must not leave credits for a later Catalog identity.
        let animations = crate::assets::edge_animation::retire_catalog(
            self.animation_resources.get_mut().unwrap_or_else(|error| error.into_inner()).take());
        let declarations = usize::from(self.common_declaration_texts.get_mut()
            .unwrap_or_else(|error| error.into_inner()).take().is_some());
        let blur = usize::from(self.camera_blur.get_mut()
            .unwrap_or_else(|error| error.into_inner()).take().is_some());
        let contacts = usize::from(self.contact_database.get_mut()
            .unwrap_or_else(|error| error.into_inner()).take().is_some());
        let manifests = self.manifests.take().map_or(0, |mut owner| owner.entry_count());
        let map_info = self.map_info.take().map_or(0, |mut owner| owner.entry_count());
        crate::loading::retire_catalog(self.identity, animations, declarations, blur, contacts, parameters, manifests, map_info);
    }
}

// Native image programs are tokenized before their source alias is emitted.
// Ignore spacing around punctuation, preserving quoted paths and word breaks.
fn image_program_key(source: &str) -> String {
    let source = source.trim().to_ascii_lowercase();
    let source = source.strip_suffix(".imageprog").unwrap_or(&source);
    let mut key = String::new();
    let mut quoted = false;
    let mut space = false;
    for c in source.chars() {
        let c = if c == '\\' { '/' } else { c };
        if !quoted && c.is_whitespace() {
            space = true;
            continue;
        }
        if space
            && !key.is_empty()
            && !"(),".contains(c)
            && !"(),".contains(key.chars().last().unwrap())
        {
            key.push(' ');
        }
        space = false;
        key.push(c);
        if c == '"' {
            quoted = !quoted;
        }
    }
    key
}

impl Catalog {
    pub(crate) fn release_parameter_block_level(&self) {
        crate::assets::parm_block::release_marker(self, 0);
    }

    pub(crate) fn release_pickable_level(&self) {
        if let Some(owner) = self.pickable_sources.get() {
            owner.lock().unwrap_or_else(|error| error.into_inner()).release_level(self);
        }
    }
    /// CPU resource purge after outgoing consumers have released their leases.
    /// The renderer's publication/fence acknowledgement precedes this call
    /// in the live client; this does not retire GPU resources on its behalf.
    pub(crate) fn release_level_resources(&self) {
        crate::edge_animation::release_level(self);
        crate::edge_skeleton::release_unused(self);
        self.release_md6_level();
        self.release_animset_level();
        self.release_parameter_block_level();
        crate::assets::entities::effects::declarations::release_marker(self, 0);
        crate::assets::fx::particle_resources::release_marker(self, 0);
        crate::assets::wwise::event_resources::release_marker(self, 0);
        crate::assets::wwise::auxiliary_resources::release_marker(self, 0);
        crate::assets::wwise::rtpc_resources::release_marker(self, 0);
        crate::assets::difficulty::release_marker(self, 0);
        crate::assets::physworld::mass::resources::release_marker(self, 0);
        crate::assets::fx::fade::release_marker(self, 0);
        crate::assets::entities::physics_material::resources::release_marker(self, 0);
        crate::assets::physworld::physics_system::release_marker(self, 0);
        self.release_pickable_level();
    }

    pub(crate) fn release_persistent_level_resources(&self) {
        crate::assets::parm_block::release_marker(self, 1);
        crate::assets::fx::particle_resources::release_marker(self, 1);
        crate::assets::wwise::event_resources::release_marker(self, 1);
        crate::assets::wwise::auxiliary_resources::release_marker(self, 1);
        crate::assets::wwise::rtpc_resources::release_marker(self, 1);
        crate::assets::physworld::mass::resources::release_marker(self, 1);
        crate::assets::fx::fade::release_marker(self, 1);
        crate::assets::entities::physics_material::resources::release_marker(self, 1);
        crate::assets::physworld::physics_system::release_marker(self, 1);
        crate::assets::entities::effects::declarations::release_marker(self, 1);
        crate::assets::inventory_item::configuration::release_persistent(self);
        crate::edge_animation::release_marker(self, crate::edge_animation::PurgeMarker::PersistentHeap);
        crate::assets::animation_rig::animset::release_scoped(self, crate::edge_animation::PurgeMarker::PersistentHeap);
        crate::assets::animation_rig::declaration::release_marker(self,
            crate::assets::animation_rig::declaration::PurgeMarker::PersistentHeap);
    }

    #[cfg(feature = "game")]
    pub(crate) fn discard_unpublished_level(&self) {
        let mut map = self.preload_map.lock().unwrap_or_else(|error| error.into_inner());
        self.release_level_resources();
        self.release_persistent_level_resources();
        let discarded = map.take();
        let descriptors = crate::loading::discard_level(self.identity());
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            eprintln!("UNPUBLISHED_LEVEL_RETIREMENT map={discarded:?} descriptors={descriptors}");
        }
    }
    /// Trim dead level AnimSet identities without dropping the session root
    /// or declarations still retained by live model/scene owners.
    pub(crate) fn release_md6_level(&self) {
        crate::assets::animation_rig::declaration::release_level(self);
    }

    pub(crate) fn release_animset_level(&self) {
        self.animset_declarations.lock().unwrap_or_else(|error| error.into_inner()).release_level(self);
    }

    /// 140066ac0..140066b8c classifies these as streamed rather than map
    /// preload requests. The load-list reader already normalizes this path.
    pub(crate) fn preloads(&self, path: &str) -> bool {
        let streamed = self.paths.search(path).ok().is_some_and(|index| self.entries[index].is_streamed());
        !streamed && ![".sbcm", ".tbcm", ".bwm"].iter().any(|suffix| path.contains(suffix))
    }

    fn entry(&self, path: &str) -> Option<&Entry> {
        use std::borrow::Cow;
        let normalized: Cow<'_, str> = if path.bytes().all(|byte| byte != b'\\' && !byte.is_ascii_uppercase()) {
            Cow::Borrowed(path)
        } else {
            let mut normalized = path.replace('\\', "/");
            normalized.make_ascii_lowercase();
            Cow::Owned(normalized)
        };
        self.paths.search(&normalized).ok().map(|index| &self.entries[index])
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

use index::archive_path;
/// One catalog per installation for the whole process: every owner reads the
/// same index instead of opening its own copy.
// Host: Windows spelling, junction and short-path aliases must identify one
// installed archive namespace. Keep only one alternate spelling, not an
// unbounded alias table, and preserve the ordinary equal-path fast route.
struct SharedCatalog {
    requested: PathBuf,
    catalog: std::sync::Arc<Catalog>,
    canonical: PathBuf,
    alias: Option<PathBuf>,
}
static SHARED: std::sync::Mutex<Option<SharedCatalog>> = std::sync::Mutex::new(None);
static NEXT_IDENTITY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Catalog {
    /// End the desktop session's shared root lease. Outstanding workers and
    /// consumers still own their Arcs; final retirement follows their release.
    pub(crate) fn release_shared(identity: u64) {
        let retired = SHARED.lock().unwrap_or_else(|e| e.into_inner())
            .take_if(|entry| entry.catalog.identity() == identity);
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            if let Some(entry) = &retired {
                eprintln!("MEMORY catalog session release: catalog={identity} matching_root=true remaining_consumers={}", std::sync::Arc::strong_count(&entry.catalog) - 1);
            } else {
                eprintln!("MEMORY catalog session release: catalog={identity} matching_root=false");
            }
        }
        // Drop outside SHARED: Catalog retirement invokes its resource owners.
        drop(retired);
    }

    /// The process's catalog of this installed directory, opened on first use.
    pub fn shared(base: &Path) -> Result<std::sync::Arc<Self>> {
        let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = shared.as_ref().filter(|entry| entry.requested == base) {
            return Ok(entry.catalog.clone());
        }
        static SEPARATE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let separate = *SEPARATE.get_or_init(|| std::env::var("DH2_DEBUG_SEPARATE_CATALOG_ALIASES").as_deref() == Ok("1"));
        if !separate {
            if let Some(entry) = shared.as_ref().filter(|entry| entry.alias.as_deref() == Some(base)) {
                return Ok(entry.catalog.clone());
            }
        }
        let canonical = std::fs::canonicalize(base)?;
        if !separate {
            if let Some(entry) = shared.as_mut().filter(|entry| entry.canonical == canonical) {
                entry.alias = Some(base.to_path_buf());
                if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                    eprintln!("CATALOG_ALIAS_REUSE catalog={} path={base:?} canonical={canonical:?}", entry.catalog.identity());
                }
                return Ok(entry.catalog.clone());
            }
        }
        let catalog = std::sync::Arc::new(Self::open(base)?);
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("CATALOG_SHARED_CREATE catalog={} path={base:?} canonical={canonical:?}", catalog.identity());
        }
        let retired = shared.replace(SharedCatalog { requested: base.to_path_buf(), catalog: catalog.clone(), canonical, alias: None });
        drop(shared);
        drop(retired);
        Ok(catalog)
    }

    pub fn open(base: &Path) -> Result<Self> {
        let retain_name_buffers = !std::env::var_os("DH2_DEBUG_SEPARATE_INDEX_NAMES").is_some_and(|v| v == "1");
        let trace = std::env::var_os("DH2_DEBUG_CATALOG_INDEX_NAMES").is_some_and(|v| v == "1");
        let started = trace.then(crate::loading::thread_cycles).flatten();
        let mut growths = [0usize; 3];
        let mut rows = 0;
        let mut entries = Vec::new();
        let mut kinds: Vec<String> = Vec::new();
        let mut image_sources = Vec::new();
        for group in 1..=4 {
            let mut capacities = [0; 3];
            index::visit_group_records(base, group, |record| {
                if trace {
                    rows += 1;
                    let read = [record.kind.capacity(), record.source.capacity(), record.path.capacity()];
                    for index in 0..3 { growths[index] += usize::from(read[index] > capacities[index]); }
                }
                let mut path = std::mem::take(&mut record.path);
                path.make_ascii_lowercase();
                if path.contains('\\') {
                    path = path.replace('\\', "/");
                }
                if record.kind.eq_ignore_ascii_case("image") && path.ends_with(".bimage7") {
                    let mut source = std::mem::take(&mut record.source);
                    source.make_ascii_lowercase();
                    if source.contains('\\') {
                        source = source.replace('\\', "/");
                    }
                    image_sources.push((source, path.clone()));
                }
                let kind = match kinds.iter().position(|kind| *kind == record.kind) {
                    Some(i) => i as u16,
                    None => {
                        kinds.push(std::mem::take(&mut record.kind));
                        (kinds.len() - 1) as u16
                    }
                };
                entries.push((
                    path,
                    Entry {
                        group,
                        header_groups: 1 << (group - 1),
                        offset: record.offset,
                        size: record.size,
                        stored_size: record.stored_size,
                        slot: record.archive_slot,
                        kind,
                        flags: record.flags,
                    },
                ));
                if !retain_name_buffers { *record = index::IndexEntry::default(); }
                if trace { capacities = [record.kind.capacity(), record.source.capacity(), record.path.capacity()]; }
            })?;
        }
        // The original map overwrote duplicate paths as later index groups
        // were read. Stable sorting preserves file order among duplicate
        // keys, and this fold keeps the final entry from that order.
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        entries.dedup_by(|later, earlier| {
            if later.0 == earlier.0 {
                let header_groups = earlier.1.header_groups | later.1.header_groups;
                earlier.1 = later.1;
                earlier.1.header_groups = header_groups;
                true
            } else {
                false
            }
        });
        let mut image_sources: Vec<_> = image_sources
            .into_iter()
            .map(|(source, path)| {
                let index = entries.binary_search_by(|(key, _)| key.cmp(&path)).unwrap();
                (source, index)
            })
            .collect();
        image_sources.sort_by(|left, right| left.0.cmp(&right.0));
        image_sources.dedup_by(|later, earlier| {
            if later.0 == earlier.0 {
                earlier.1 = later.1;
                true
            } else {
                false
            }
        });
        let mut image_programs = image_sources
            .iter()
            .filter(|(s, _)| s.ends_with(".imageprog"))
            .map(|(s, i)| (image_program_key(s), *i))
            .collect::<Vec<_>>();
        image_programs.sort();
        image_programs.dedup();
        let packed = |pairs: Vec<(String, usize)>| {
            let indices = pairs.iter().map(|&(_, i)| i as u32).collect();
            let names = if retain_name_buffers {
                Names::new(pairs.into_iter().map(|(name, _)| name))
            } else {
                Names::new(pairs.iter().map(|(name, _)| name.as_str()))
            };
            (names, indices)
        };
        let paths = if retain_name_buffers {
            Names::new(entries.iter_mut().map(|(path, _)| std::mem::take(path)))
        } else {
            Names::new(entries.iter().map(|(path, _)| path.as_str()))
        };
        let catalog = Self {
            identity: NEXT_IDENTITY.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            animset_declarations: Default::default(),
            md6_declarations: Default::default(),
            md6_loading: Default::default(),
            manifests: Default::default(),
            map_info: Default::default(),
            language_table: Default::default(),
            preload_bootstrap: Default::default(),
            preload_map: Default::default(),
            common_declaration_texts: Default::default(),
            camera_blur: Default::default(),
            contact_database: Default::default(),
            animation_resources: Default::default(),
            skeleton_resources: Default::default(),
            parameter_blocks: Default::default(),
            mass_resources: Default::default(),
            component_types: Default::default(),
            material_constants: Default::default(),
            physics_material_resources: Default::default(),
            fade_resources: Default::default(),
            physics_system_resources: Default::default(),
            common_models: Default::default(),
            difficulty_definitions: Default::default(),
            difficulty_presets: Default::default(),
            sound_rtpcs: Default::default(),
            sound_auxiliary_buses: Default::default(),
            sound_events: Default::default(),
            kiscule_resources: Default::default(),
            particle_resources: Default::default(),
            render_parameters: Default::default(),
            shader_indices: Default::default(),
            pickable_sources: Default::default(),
            containers: Default::default(),
            header_order: Default::default(),
            kinds,
            base: base.to_path_buf(),
            paths,
            entries: entries.into_iter().map(|(_, entry)| entry).collect(),
            image_sources: packed(image_sources),
            image_programs: packed(image_programs),
            overlay: crate::platform::home::layers().files,
        };
        if trace {
            let cycles = started.zip(crate::loading::thread_cycles()).map_or(0, |(start, end)| end.saturating_sub(start));
            eprintln!("CATALOG_INDEX_NAMES retained_scratch={retain_name_buffers} rows={rows} kind_growths={} source_growths={} path_growths={} selected={} name_bytes={} cycles={cycles}",
                growths[0], growths[1], growths[2], catalog.paths.len(), catalog.paths.text.len());
            catalog.report_index_identity();
        }
        Ok(catalog)
    }

    fn report_index_identity(&self) {
        let mut hash = blake3::Hasher::new();
        let name = |hash: &mut blake3::Hasher, value: &str| {
            hash.update(&(value.len() as u64).to_le_bytes());
            hash.update(value.as_bytes());
        };
        for (path, entry) in self.paths.iter().zip(&self.entries) {
            name(&mut hash, path);
            name(&mut hash, &self.kinds[usize::from(entry.kind)]);
            hash.update(&[entry.group, entry.header_groups]);
            hash.update(&entry.offset.to_le_bytes());
            hash.update(&entry.size.to_le_bytes());
            hash.update(&entry.stored_size.to_le_bytes());
            hash.update(&entry.slot.to_le_bytes());
            hash.update(&entry.flags.to_le_bytes());
        }
        for (names, indices) in [&self.image_sources, &self.image_programs] {
            hash.update(&(names.len() as u64).to_le_bytes());
            for (source, &index) in names.iter().zip(indices) {
                name(&mut hash, source);
                hash.update(&index.to_le_bytes());
            }
        }
        eprintln!("CATALOG_INDEX_IDENTITY digest={} image_aliases={} program_aliases={}",
            hash.finalize(), self.image_sources.0.len(), self.image_programs.0.len());
    }

    /// The overlay file replacing `path`, if any.
    fn overlay_file(&self, path: &str) -> Option<&Path> {
        if self.overlay.is_empty() { return None; }
        let normalized = path.replace('\\', "/").to_ascii_lowercase();
        self.overlay
            .binary_search_by(|(key, _)| key.as_str().cmp(&normalized))
            .ok()
            .map(|i| self.overlay[i].1.as_path())
    }

    /// An active overlay can change while a level is loading.
    pub(crate) fn has_overlays(&self) -> bool { !self.overlay.is_empty() }

    /// An overlay can change independently of the installed archive index.
    pub(crate) fn has_overlay_file(&self, path: &str) -> bool {
        self.overlay_file(path).is_some()
    }

    /// Whether an editable layer can replace a declaration in this namespace.
    pub(crate) fn has_overlay_declaration(&self, prefix: &str) -> bool {
        self.overlay.iter().any(|(path, _)| path.starts_with(prefix) && path.ends_with(".decl"))
    }

    /// A compiled image and its separate mip members form one cached identity.
    /// Layer names are normalized and sorted when the catalog is opened.
    pub(crate) fn image_has_overlay(&self, path: &str) -> bool {
        if self.overlay.is_empty() { return false; }
        let normalized = path.replace('\\', "/").to_ascii_lowercase();
        let path = normalized.rsplit_once("_mip").filter(|(_, mip)|
            !mip.is_empty() && mip.bytes().all(|byte| byte.is_ascii_digit()))
            .map_or(normalized.as_str(), |(base, _)| base);
        let start = self.overlay.partition_point(|(key, _)| key.as_str() < path);
        self.overlay[start..].iter().take_while(|(key, _)| key.starts_with(path)).any(|(key, _)| {
            key.as_str() == path || key.strip_prefix(path).and_then(|tail| tail.strip_prefix("_mip"))
                .is_some_and(|mip| !mip.is_empty() && mip.bytes().all(|byte| byte.is_ascii_digit()))
        })
    }

    /// Resource paths of the archives and the overlay, sorted and unique.
    fn all_paths<'a>(&'a self, keep: impl Fn(&str) -> bool + 'a) -> Vec<String> {
        let mut paths: Vec<String> = self
            .paths
            .iter()
            .chain(self.overlay.iter().map(|(path, _)| path.as_str()))
            .filter(|path| keep(path))
            .map(str::to_string)
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn contains(&self, path: &str) -> bool {
        self.overlay_file(path).is_some() || self.entry(path).is_some()
    }

    /// Decoded size recorded by the installed resource index (an overlay
    /// file's own size).
    pub fn resource_size(&self, path: &str) -> Option<usize> {
        if let Some(file) = self.overlay_file(path) {
            return std::fs::metadata(file).ok().map(|m| m.len() as usize);
        }
        self.entry(path).map(|entry| entry.size as usize)
    }

    /// Data archive holding the selected payload. Its slot is not the native
    /// primary header container ID returned by 140064590.
    pub(super) fn resource_container(&self, path: &str) -> Option<(u8, u16)> {
        self.entry(path).map(|entry| (entry.group, entry.slot))
    }

    /// 140064590 returns the first header table containing the path, in the
    /// order mounted by 1400658b0. Preserve all memberships even when a later
    /// table supplies the payload selected by the host catalog.
    pub(crate) fn resource_header_container(&self, path: &str) -> std::result::Result<Option<u16>, String> {
        let Some(entry) = self.entry(path) else { return Ok(None) };
        let order = self.header_order.get_or_init(|| index::primary_header_groups(&self.base)
            .map_err(|error| error.to_string())).as_ref().map_err(Clone::clone)?;
        order.iter().position(|group| entry.header_groups & (1 << (group - 1)) != 0)
            .map(|index| u16::try_from(index).map_err(|_| "primary header container ID exceeds 16 bits".into()))
            .transpose()
    }

    /// Resolve the source->compiled resource mapping stored by the game itself.
    pub fn image_resource(&self, source: &str) -> Option<&str> {
        // Native idStr slash normalization140140da0/140140de0 treats both
        // Windows separators alike before resource/image-source lookup.
        let source = source.replace('\\', "/").to_ascii_lowercase();
        let index = self.image_sources.0.search(&source).ok()?;
        Some(self.paths.get(self.image_sources.1[index] as usize))
    }

    /// Resolve any compiled image program through authoritative index aliases;
    /// no guessed compiled filename or replacement procedural image is used.
    pub fn image_program_resource(&self, source: &str) -> Option<&str> {
        let key = image_program_key(source);
        let (names, indices) = &self.image_programs;
        let i = names.partition_point(|s| s < key.as_str());
        if i >= names.len() || names.get(i) != key || (i + 1 < names.len() && names.get(i + 1) == key) {
            return None;
        }
        Some(self.paths.get(indices[i] as usize))
    }

    pub fn paths_with_suffix(&self, suffix: &str) -> Vec<String> {
        let suffix = suffix.to_ascii_lowercase();
        self.all_paths(move |path| path.ends_with(&suffix))
    }

    pub fn paths_containing(&self, needle: &str) -> Vec<String> {
        let needle = needle.to_ascii_lowercase();
        self.all_paths(move |path| path.contains(&needle))
    }

    /// A newly opened catalog may have a different overlay even for the same installation.
    pub(crate) fn identity(&self) -> u64 { self.identity }

    /// The installed `base` folder the catalog reads.
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// The index kind of `path`.
    pub fn kind(&self, path: &str) -> Option<&str> {
        self.entry(path).map(|e| self.kinds[e.kind as usize].as_str())
    }

    pub fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        self.read_cancellable(path, None)
    }

    pub(crate) fn read_cancellable(&self, path: &str, cancelled: Option<&dyn Fn() -> bool>) -> Result<Option<Vec<u8>>> {
        let mut output = Vec::new();
        Ok(self.read_member(path, &mut output, cancelled)?.then_some(output))
    }

    /// Reuse the caller's decoded storage across independent member reads.
    /// A missing or failed member leaves the buffer empty.
    pub(crate) fn read_into(&self, path: &str, output: &mut Vec<u8>) -> Result<bool> {
        self.read_member(path, output, None)
    }

    fn read_member(&self, path: &str, output: &mut Vec<u8>, cancelled: Option<&dyn Fn() -> bool>) -> Result<bool> {
        output.clear();
        check_read_cancelled(cancelled)?;
        let entry = self.entry(path);
        let result = if let Some(file) = self.overlay_file(path) {
            if output.capacity() == 0 && cancelled.is_none() {
                *output = std::fs::read(file)?;
                Ok(true)
            } else {
                read_member_bytes(&mut File::open(file)?, output, cancelled).map(|()| true).map_err(Into::into)
            }
        } else if let Some(entry) = entry {
            let mut reader = self.containers.acquire(&self.base, entry.group, entry.slot)?;
            let length = reader.length;
            read_file_into(&mut reader.file, length, path, entry, output, cancelled).map(|()| true)
        } else {
            Ok(false)
        };
        let result = result.and_then(|found| {
            check_read_cancelled(cancelled)?;
            Ok(found)
        });
        // A failed acquisition cannot advance loading progress. Record only
        // actual payloads after archive/overlay IO and decompression succeed.
        if matches!(&result, Ok(true)) {
            self.record_payload_read(entry, path);
        } else if result.is_err() {
            output.clear();
        }
        result
    }

    fn record_payload_read(&self, entry: Option<&Entry>, path: &str) {
        let Some(entry) = entry else { return };
        let kind = &self.kinds[entry.kind as usize];
        // The map preloader (140066ac0) excludes these mips. Their texture
        // owner accounts for publication and eviction independently.
        const MIP_EXTENSION: &str = "bimage7_mip";
        if entry.is_streamed() && kind == "image"
            && path.rsplit_once('.').and_then(|(_, extension)| extension.get(..MIP_EXTENSION.len()))
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(MIP_EXTENSION))
        {
            static RETAIN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if !*RETAIN.get_or_init(|| {
                std::env::var_os("DH2_DEBUG_RETAIN_STREAMED_MIP_HISTORY").is_some_and(|value| value == "1")
            }) {
                return;
            }
        }
        crate::loading::read_catalog(self.identity, kind, path);
    }

    /// Borrow a bounded, buffered stream over an installed member. The
    /// container lease remains owned here until the consumer returns, so a
    /// compressed member need not be expanded into a whole-file allocation.
    pub(crate) fn with_stream<T>(&self, path: &str, consume: impl FnOnce(&mut dyn Read, usize) -> std::result::Result<T, String>) -> std::result::Result<Option<T>, String> {
        self.with_stream_cancellable(path, None, consume)
    }

    pub(crate) fn with_stream_cancellable<T>(&self, path: &str, cancelled: Option<&dyn Fn() -> bool>,
        consume: impl FnOnce(&mut dyn Read, usize) -> std::result::Result<T, String>,
    ) -> std::result::Result<Option<T>, String> {
        check_read_cancelled(cancelled).map_err(|error| error.to_string())?;
        let consume = |input: &mut dyn Read, size| {
            if let Some(cancelled) = cancelled {
                consume(&mut CancellableMember { input, cancelled }, size)
            } else {
                consume(input, size)
            }
        };
        let entry = self.entry(path);
        let value = if let Some(file) = self.overlay_file(path) {
            let input = File::open(file).map_err(|e| e.to_string())?;
            let size = input.metadata().map_err(|e| e.to_string())?.len() as usize;
            let mut input = std::io::BufReader::with_capacity(65536, input);
            consume(&mut input, size)?
        } else {
            let Some(entry) = entry else { return Ok(None); };
            let mut reader = self.containers.acquire(&self.base, entry.group, entry.slot).map_err(|e| e.to_string())?;
            validate_extent(path, entry, reader.length).map_err(|e| e.to_string())?;
            reader.file.seek(SeekFrom::Start(entry.offset)).map_err(|e| e.to_string())?;
            let input = (&mut reader.file).take(u64::from(entry.stored_size));
            if entry.size == entry.stored_size {
                let mut input = std::io::BufReader::with_capacity(65536, input);
                consume(&mut input, entry.size as usize)?
            } else {
                let mut input = std::io::BufReader::with_capacity(65536, ZlibDecoder::new(input));
                let value = consume(&mut input, entry.size as usize)?;
                check_read_cancelled(cancelled).map_err(|error| error.to_string())?;
                // read_file probes beyond the indexed decoded size, forcing
                // trailer validation and detecting oversized output.
                let mut extra = [0u8; 1];
                if input.read(&mut extra).map_err(|e| e.to_string())? != 0 {
                    return Err(format!("{path}: decoded data exceeds indexed size {}", entry.size));
                }
                value
            }
        };
        check_read_cancelled(cancelled).map_err(|error| error.to_string())?;
        self.record_payload_read(entry, path);
        Ok(Some(value))
    }

    /// Read only the start of an installed resource. This is useful for large
    /// indexed files whose header tables precede a much larger payload.
    pub fn read_prefix(&self, path: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        if let Some(file) = self.overlay_file(path) {
            let mut output = Vec::new();
            File::open(file)?.take(limit as u64).read_to_end(&mut output)?;
            return Ok(Some(output));
        }
        let Some(entry) = self.entry(path) else {
            return Ok(None);
        };
        let mut reader = self.containers.acquire(&self.base, entry.group, entry.slot)?;
        validate_extent(path, entry, reader.length)?;
        let file = &mut reader.file;
        file.seek(SeekFrom::Start(entry.offset))?;
        let mut output = Vec::with_capacity((entry.size as usize).min(limit));
        if entry.size == entry.stored_size {
            file.take(entry.stored_size as u64)
                .take(limit as u64)
                .read_to_end(&mut output)?;
        } else {
            let decoder = ZlibDecoder::new(file.take(entry.stored_size as u64));
            decoder.take(limit as u64).read_to_end(&mut output)?;
        }
        Ok(Some(output))
    }
}

fn read_location(base: &Path, path: &str, entry: &Entry) -> Result<Vec<u8>> {
    let archive = archive_path(base, entry.group, entry.slot)?;
    let mut file = File::open(&archive)?;
    let archive_size = file.metadata()?.len();
    read_file(&mut file, archive_size, path, entry)
}

fn validate_extent(path: &str, entry: &Entry, archive_size: u64) -> Result<()> {
    if entry
        .offset
        .checked_add(u64::from(entry.stored_size))
        .is_none_or(|end| end > archive_size)
    {
        return Err(invalid(format!("{path}: indexed resource exceeds archive extent")).into());
    }
    Ok(())
}

fn read_file(file: &mut File, archive_size: u64, path: &str, entry: &Entry) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    read_file_into(file, archive_size, path, entry, &mut output, None)?;
    Ok(output)
}

fn read_file_into(file: &mut File, archive_size: u64, path: &str, entry: &Entry, output: &mut Vec<u8>,
    cancelled: Option<&dyn Fn() -> bool>,
) -> Result<()> {
    validate_extent(path, entry, archive_size)?;
    file.seek(SeekFrom::Start(entry.offset))?;
    // Native worlds exceed 128 MiB. Bound reads by the indexed archive
    // extent and decoded size, rather than an unrelated texture-size cap.
    // Stream decompression so loading a world does not retain a second
    // allocation for its entire compressed representation.
    let input = file.take(u64::from(entry.stored_size));
    // The decoded size plus the one probe byte that detects an oversized
    // stream, so read_to_end never regrows and copies the buffer.
    output.try_reserve_exact(entry.size as usize + 1)?;
    if entry.size == entry.stored_size {
        read_member_bytes(&mut input.take(u64::from(entry.size) + 1), output, cancelled)?;
    } else {
        read_member_bytes(&mut ZlibDecoder::new(input).take(u64::from(entry.size) + 1), output, cancelled)?;
    }
    if output.len() != entry.size as usize {
        return Err(invalid(format!(
            "{}: expected {} bytes, decoded {}",
            path,
            entry.size,
            output.len()
        ))
        .into());
    }
    Ok(())
}

fn check_read_cancelled(cancelled: Option<&dyn Fn() -> bool>) -> io::Result<()> {
    if cancelled.is_some_and(|cancelled| cancelled()) {
        // read_exact and read_to_end retry ErrorKind::Interrupted.
        return Err(io::Error::other("archive acquisition cancelled"));
    }
    Ok(())
}

struct CancellableMember<'a, R: Read + ?Sized> {
    input: &'a mut R,
    cancelled: &'a dyn Fn() -> bool,
}

impl<R: Read + ?Sized> Read for CancellableMember<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        check_read_cancelled(Some(self.cancelled))?;
        let size = output.len().min(64 * 1024);
        self.input.read(&mut output[..size])
    }
}

fn read_member_bytes(input: &mut impl Read, output: &mut Vec<u8>, cancelled: Option<&dyn Fn() -> bool>) -> io::Result<()> {
    if let Some(cancelled) = cancelled {
        CancellableMember { input, cancelled }.read_to_end(output)?;
    } else {
        input.read_to_end(output)?;
    }
    Ok(())
}

/// Interrupt both installed member readers before they can publish read
/// history, then compare their successful output in a fresh lease.
#[cfg(feature = "game")]
pub(crate) fn observe_member_cancellation(base: &Path, path: &str) -> Result<()> {
    use std::cell::Cell;
    let catalog = Catalog::open(base)?;
    let entry = catalog.entry(path).ok_or("installed member missing")?;
    if entry.size <= 2 * 64 * 1024 || catalog.has_overlay_file(path) {
        return Err("select a large installed archive member".into());
    }
    let kind = &catalog.kinds[entry.kind as usize];
    let credited = || crate::loading::catalog_read_recorded(catalog.identity(), kind, path);
    let checks = Cell::new(0);
    let mut buffer = Vec::new();
    let cancelled = catalog.read_member(path, &mut buffer, Some(&|| {
        let next = checks.get() + 1;
        checks.set(next);
        next == 4
    }));
    let member_cancelled = cancelled.is_err_and(|error| error.to_string() == "archive acquisition cancelled");
    let partial_discarded = buffer.is_empty();
    let cancelled_before_credit = !credited();

    let closed = Cell::new(false);
    let prefix_bytes = 32 * 1024;
    let streamed = catalog.with_stream_cancellable(path, Some(&|| closed.get()), |input, _| {
        let mut prefix = vec![0; prefix_bytes];
        input.read_exact(&mut prefix).map_err(|error| error.to_string())?;
        closed.set(true);
        let mut next = [0; 1];
        input.read_exact(&mut next).map_err(|error| error.to_string())
    });
    let stream_cancelled = streamed.is_err_and(|error| error == "archive acquisition cancelled");
    let stream_uncredited = !credited();
    let control = catalog.read(path)?.ok_or("installed member disappeared")?;
    let retry = catalog.read_cancellable(path, Some(&|| false))?.ok_or("retry lost member")?;
    let streamed_retry = catalog.with_stream_cancellable(path, Some(&|| false), |input, size| {
        let mut bytes = vec![0; size];
        input.read_exact(&mut bytes).map_err(|error| error.to_string())?;
        Ok(bytes)
    })?;
    let exact = retry == control && retry.len() == entry.size as usize
        && streamed_retry.as_deref() == Some(retry.as_slice());
    let retry_credited = credited();
    let checksum = blake3::hash(&retry);
    println!("ARCHIVE_MEMBER_CANCELLATION path={path} stored={} decoded={} prefix_bytes={prefix_bytes} check_count={} member_cancelled={member_cancelled} partial_discarded={partial_discarded} cancelled_before_credit={cancelled_before_credit} stream_cancelled={stream_cancelled} stream_uncredited={stream_uncredited} exact={exact} retry_credited={retry_credited} checksum={checksum}",
        entry.stored_size, entry.size, checks.get());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn be16_bytes(value: u16) -> [u8; 2] {
        value.to_be_bytes()
    }

    fn be32_bytes(value: u32) -> [u8; 4] {
        value.to_be_bytes()
    }

    fn be64_bytes(value: u64) -> [u8; 8] {
        value.to_be_bytes()
    }

    fn index_entry(path: &str, offset: u64, size: u32) -> Vec<u8> {
        typed_index_entry("kind", "source", path, offset, size)
    }

    fn typed_index_entry(kind: &str, source: &str, path: &str, offset: u64, size: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(be32_bytes(0));
        for value in [kind, source, path] {
            bytes.extend((value.len() as u32).to_le_bytes());
            bytes.extend(value.as_bytes());
        }
        bytes.extend(be64_bytes(offset));
        bytes.extend(be32_bytes(size));
        bytes.extend(be32_bytes(size));
        bytes.extend([0; 6]);
        bytes.extend(be16_bytes(0));
        bytes.extend(be16_bytes(0));
        bytes
    }

    fn write_index(path: &Path, entries: &[Vec<u8>]) {
        let payload_size: usize = 4 + entries.iter().map(Vec::len).sum::<usize>();
        let mut file = File::create(path).unwrap();
        file.write_all(b"\x05SER").unwrap();
        file.write_all(&be32_bytes(payload_size as u32)).unwrap();
        file.write_all(&[0; 24]).unwrap();
        file.write_all(&be32_bytes(entries.len() as u32)).unwrap();
        for entry in entries {
            file.write_all(entry).unwrap();
        }
    }

    #[test]
    fn resource_slots_are_restricted() {
        assert!(archive_path(Path::new("base"), 2, 8192)
            .unwrap()
            .ends_with("shared_2_3.sharedrsc"));
        assert!(archive_path(Path::new("base"), 1, 8192).is_err());
    }

    #[test]
    fn level_resolution_rejects_ambiguity_and_uses_indexed_physics_names() {
        use crate::assets::level::LevelResources;
        let root = std::env::temp_dir().join(format!("dh2-level-resources-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let paths = [
            "maps/a/sample_lowchaos/sample_lowchaos.bwm",
            "maps/b/sample_lowchaos/sample_lowchaos.bwm",
            "maps/a/sample_lowchaos.entities",
            "maps/a/sample_lowchaos/sample_lowchaos.tome",
            "generated/physworld/maps/a/sample_lowchaos/sample.bphysworld",
        ];
        let entries: Vec<_> = paths.iter().map(|path| index_entry(path, 0, 0)).collect();
        write_index(&root.join("game1_patch.index"), &entries);
        for group in 2..=4 {
            write_index(&root.join(format!("game{group}_patch.index")), &[]);
        }
        let catalog = Catalog::open(&root).unwrap();
        assert!(LevelResources::resolve(&catalog, "sample_lowchaos").is_err());
        assert!(LevelResources::resolve(&catalog, "missing").is_err());
        let level = LevelResources::resolve(&catalog, paths[0]).unwrap();
        assert_eq!(level.entities, paths[2]);
        assert_eq!(level.tome, paths[3]);
        assert_eq!(level.physics.as_deref(), Some(paths[4]));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sorted_catalog_preserves_case_insensitive_lookup_and_later_group_override() {
        let root = std::env::temp_dir().join(format!(
            "dh2-catalog-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();

        fs::write(root.join("game1.resources"), b"first").unwrap();
        fs::write(root.join("game4.resources"), b"later").unwrap();
        write_index(
            &root.join("game1_patch.index"),
            &[
                index_entry("Thing/Asset.bin", 0, 5),
                typed_index_entry(
                    "image",
                    "tree$nomips.tga",
                    "generated/image/tree$nomips.tga.bimage7",
                    0,
                    5,
                ),
                typed_index_entry(
                    "image",
                    "ipr_constantcolor( 0, 0, 0, 1).imageprog",
                    "generated/image/ipr_constantcolor__0__0__0__1_.bimage7",
                    0,
                    5,
                ),
            ],
        );
        write_index(&root.join("game2_patch.index"), &[]);
        write_index(&root.join("game3_patch.index"), &[]);
        write_index(
            &root.join("game4_patch.index"),
            &[index_entry("thing/asset.bin", 0, 5)],
        );

        let catalog = Catalog::open(&root).unwrap();
        assert_eq!(catalog.len(), 3);
        assert!(catalog.contains("THING/ASSET.BIN"));
        assert!(catalog.contains("THING\\ASSET.BIN"));
        assert_eq!(
            catalog.image_resource("TREE$nomips.TGA"),
            Some("generated/image/tree$nomips.tga.bimage7")
        );
        assert_eq!(
            catalog.image_resource("ipr_constantColor( 0, 0, 0, 1).imageprog"),
            Some("generated/image/ipr_constantcolor__0__0__0__1_.bimage7")
        );
        assert_eq!(catalog.image_resource("tree.tga"), None);
        assert_eq!(
            catalog.image_program_resource("ipr_constantColor(0,0,0,1)"),
            Some("generated/image/ipr_constantcolor__0__0__0__1_.bimage7")
        );
        assert_ne!(
            image_program_key("ipr_test(\"a b\",1)"),
            image_program_key("ipr_test(\"ab\",1)")
        );
        assert_eq!(
            image_program_key("ipr_test(\"folder\\image.tga\",1)"),
            image_program_key("ipr_test(\"folder/image.tga\",1)")
        );
        assert_eq!(
            catalog.read("Thing/Asset.bin").unwrap(),
            Some(b"later".to_vec())
        );

        fs::write(root.join("game4.resources"), b"no").unwrap();
        assert!(catalog
            .read("thing/asset.bin")
            .unwrap_err()
            .to_string()
            .contains("archive extent"));

        drop(catalog);
        fs::remove_dir_all(root).unwrap();
    }
}
