//! Ammo44dd00/62b240 and upgrade61a2b0 child entities. Geometry, rig
//! and clips stay with their canonical format owners; only actual declared
//! components select an animator. Native61a650 posts AnimBasic input zero.
use super::{SourceRead, check_active};
use crate::assets::{animation_rig::BasicRig, archive::Catalog, bmd6, bmodel, entities};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

#[derive(Clone)]
pub(crate) struct Descriptor {
    pub entity: String,
    pub class: String,
    pub model: String,
    pub cast_shadows: bool,
    pub placement: entities::PlacedModel,
    pub geometry: Geometry,
    /// Entitydef spelling, or the actual idEntity class for61a2b0 direct models.
    pub name_source: String,
}

#[derive(Clone)]
pub(crate) enum Geometry {
    Rigid(Arc<bmodel::ModelSource>),
    Animated {
        rig: Arc<BasicRig>,
        model: Arc<bmd6::ModelInfo>,
    },
}
impl Geometry {
    pub(crate) fn bounds(&self) -> [[f32; 3]; 2] {
        match self {
            Self::Rigid(model) => model.authored_bounds,
            Self::Animated { model, .. } => model.authored_bounds,
        }
    }
}
impl Descriptor {
    pub(crate) fn material_requests(&self) -> Vec<String> {
        let mut paths = Vec::new();
        let mut add = |index, path: &str, empty: bool| {
            let path = self
                .placement
                .custom_materials
                .get(index)
                .and_then(Option::as_deref)
                .unwrap_or(path);
            if !empty && !crate::material::is_null_material(path) {
                paths.push(path.to_owned());
            }
        };
        match &self.geometry {
            Geometry::Rigid(model) => {
                for (index, surface) in model.surfaces.iter().enumerate() {
                    add(
                        index,
                        &surface.material_path,
                        surface.positions.is_empty() || surface.indices.is_empty(),
                    );
                }
            }
            Geometry::Animated { model, .. } => {
                for (index, surface) in model.surfaces.iter().enumerate() {
                    add(
                        index,
                        &surface.material_path,
                        surface.positions.is_empty() || surface.indices.is_empty(),
                    );
                }
            }
        }
        paths.sort_unstable();
        paths.dedup();
        paths
    }
}

/// Geometry projection for an entity already admitted by its actual runtime
/// class owner. This does not reinterpret a projectile/chunk as idEntity and
/// never admits its components. All source geometry and clips use their
/// existing format registries, not a second world-model cache.
pub(crate) fn project_world(catalog:&Catalog, definition:&entities::Definition, animation:Option<&str>, cancelled:Option<&dyn Fn()->bool>)->Result<Arc<Descriptor>,String>{
    check_active(cancelled)?;
    let mut source=SourceRead{catalog,cancelled,texts:HashMap::new()};
    if let Some(tree)=animation{
        let path=format!("generated/decls/animtree/{tree}.decl");
        let text=source.read(&path)?.ok_or_else(||format!("world actor animation tree missing: {path}"))?;
        super::tree::anim_basic_only(&text)?;
    }
    let mut fields=definition.fields.clone();
    let edit=entities::scene_motion::child(&fields,"edit");
    let info=entities::scene_motion::child(edit,"renderModelInfo");
    let model=entities::scene_motion::atom(info,"model").ok_or("world actor has no authored render model")?.to_owned();
    let cast_shadows=match entities::scene_motion::atom(info,"m_castShadows"){None|Some("true")=>true,Some("false")=>false,Some(value)=>return Err(format!("invalid world model cast-shadow flag {value}"))};
    // Runtime spawn parameters supply position separately. The projection
    // only needs the ordinary placement parser for authored scale/materials.
    for (key,value) in &mut fields {if key=="edit" {if let entities::Value::Scope(edit)=value {if !edit.iter().any(|(key,_)|key=="spawnPosition"){edit.push(("spawnPosition".into(),entities::Value::Scope(Vec::new())));}}}}
    let projected=entities::Definition{kind:definition.kind.clone(),class:definition.class.clone(),id:definition.id.clone(),fields};
    let placement=entities::model_descriptor(&projected,|path|source.read(path))?.ok_or("world actor render model descriptor is absent")?;
    let geometry=if model.ends_with(".lwo") && animation.is_none(){
        Geometry::Rigid(Arc::new(bmodel::acquire(catalog,&placement.resource_path,bmodel::CommonModels::shared(catalog))?.ok_or_else(||format!("world model missing: {}",placement.resource_path))?))
    }else if model.ends_with(".md6") && animation.is_some(){
        let rig=Arc::new(BasicRig::load_cancellable(catalog,&model,cancelled)?);
        Geometry::Animated{rig,model:bmd6::ModelSources::new(catalog).acquire(catalog,&placement.resource_path)?}
    }else{return Err(format!("world model {model}: actual animation component pairing is unsupported"));};
    check_active(cancelled)?;
    Ok(Arc::new(Descriptor{entity:definition.id.clone(),class:definition.class.clone(),model,cast_shadows,placement,geometry,name_source:definition.id.clone()}))
}

/// 1419f8ee0 / upgradeInfo+68. Native61a650 filters this exact value before
/// issuing input-index-zero commands; STATIC is the constructor's zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Animation {
    Static = 0,
    Looping = 1,
    Charge = 2,
    Fire = 3,
    Reload = 4,
    Magazine = 5,
    Firelock = 6,
}
impl Animation {
    fn read(value: Option<&str>) -> Result<Self, String> {
        Ok(match value.unwrap_or("STATIC") {
            "STATIC" | "0" => Self::Static,
            "LOOPING_ANIMATION" | "1" => Self::Looping,
            "LOOPING_ANIMATION_ON_CHARGE" | "2" => Self::Charge,
            "ANIMATE_ON_FIRE" | "3" => Self::Fire,
            "ANIMATE_ON_RELOAD" | "4" => Self::Reload,
            "ANIMATE_MAGAZINE" | "5" => Self::Magazine,
            "ANIMATE_FIRELOCK" | "6" => Self::Firelock,
            value => return Err(format!("invalid equipment attachment animation {value}")),
        })
    }
}
pub(crate) struct Upgrade {
    pub declaration: String,
    /// Native upgrade record +10/+18: static-model entity and entitydef
    /// entity are separate optional children, not mutually exclusive.
    pub attachments: [Option<Arc<Descriptor>>; 2],
    pub tag: String,
    pub animation: Animation,
    pub ammunition: Vec<AmmoUpgrade>,
}

///141703ee0/638190: only records matching the selected inventory ammo item
///participate; ordered projectile overrides precede the remaining records.
#[derive(Clone, Debug)]
pub(crate) struct AmmoUpgrade {
    pub item: String,
    pub static_model: Option<String>,
    pub materials: Vec<Option<String>>,
    pub projectile: Option<String>,
    pub grenade: Option<String>,
    pub mine: Option<String>,
    pub muzzle_fx: Option<String>,
    pub explosion_fx: Option<String>,
    pub flags: u32,
}
impl AmmoUpgrade {
    fn read(fields: &[(String, entities::Value)]) -> Result<Self, String> {
        use entities::scene_motion::{atom, child};
        let text = |name| atom(fields, name).filter(|value| !value.is_empty() && *value != "NULL").map(str::to_owned);
        let mut flags = 0;
        //Accessor/setter pairs registered1417037a0 write these14bits.
        for (bit, name) in ["m_burstMode", "m_firelockMode", "m_chargedMode", "m_spreadReduced", "m_spreadUpgraded",
            "m_noBullet", "m_canPinLimbs", "m_ignoreArmor", "m_ignoreKnockedOffArmor", "m_passThroughNPCs",
            "m_electrifiedDamage", "m_bounceOnceInsteadOfBreaking", "m_explodeOnHitNPC", "m_overridePowerWheelDesc"].into_iter().enumerate() {
            match atom(child(fields, "m_flags"), name) {
                Some("true" | "1") => flags |= 1 << bit,
                None | Some("false" | "0") => {},
                Some(value) => return Err(format!("invalid ammo upgrade flag {name}={value}")),
            }
        }
        //637e40 replaces the whole indexed material array. Null entries are
        //real surface slots, so compacting them assigns later materials to
        //the wrong surface.
        let overrides = child(fields, "m_materialOverrides");
        let count = atom(overrides, "num").map(str::parse::<usize>).transpose()
            .map_err(|_| "invalid ammo material override count")?.unwrap_or(0);
        let mut materials = vec![None; count];
        for (key, value) in overrides {
            let Some(index) = key.strip_prefix("m_materialOverrides[").or_else(|| key.strip_prefix("item[")).and_then(|key| key.strip_suffix(']')) else { continue };
            let index: usize = index.parse().map_err(|_| "invalid ammo material override index")?;
            let slot = materials.get_mut(index).ok_or("ammo material override index exceeds count")?;
            let entities::Value::Atom(value) = value else { return Err("ammo material override is not a reference".into()) };
            let value = value.trim_matches('"');
            *slot = (!value.is_empty() && !value.eq_ignore_ascii_case("NULL")).then(|| value.to_owned());
        }
        Ok(Self { item: text("m_targetAmmoItem").ok_or("ammo upgrade has no target inventory item")?,
            static_model: text("m_staticModelOverride"), materials,
            projectile: text("m_projectileDeclOverride"), grenade: text("m_grenadeDeclOverride"), mine: text("m_mineDeclOverride"),
            muzzle_fx: text("m_muzzleWorldFxOverride"), explosion_fx: text("m_explosionFxOverride"), flags })
    }
}

///637e40 is shared by the visible loaded-ammo child and the fired entity
///(639970). Retain the actual placement/class while replacing only authored
///render geometry and the last nonempty whole material array.
pub(crate) fn ammo_model_variant(catalog: &Catalog, base: Arc<Descriptor>, upgrades: &[AmmoUpgrade],
    cancelled: Option<&dyn Fn() -> bool>) -> Result<Arc<Descriptor>, String> {
    let model = upgrades.iter().rev().find_map(|upgrade| upgrade.static_model.as_deref());
    let materials = upgrades.iter().rev().find(|upgrade| !upgrade.materials.is_empty()).map(|upgrade| &upgrade.materials);
    if model.is_none() && materials.is_none() { return Ok(base); }
    check_active(cancelled)?;
    let mut descriptor = (*base).clone();
    if let Some(model) = model {
        descriptor = (*static_model_variant(catalog, &descriptor.entity, &descriptor.class,
            Some(Arc::new(descriptor.clone())), model, cancelled)?).clone();
    }
    if let Some(materials) = materials { descriptor.placement.custom_materials = materials.clone(); }
    check_active(cancelled)?;
    Ok(Arc::new(descriptor))
}

/// The actual static-model format owner used by2679a0/87bcb0 and637e40.
/// Existing actor placement/class remains its own; no entitydef is fabricated.
pub(crate) fn static_model_variant(
    catalog: &Catalog, entity: &str, class: &str, base: Option<Arc<Descriptor>>,
    model: &str, cancelled: Option<&dyn Fn() -> bool>,
) -> Result<Arc<Descriptor>, String> {
    check_active(cancelled)?;
    let resource = if let Some(stem) = model.strip_suffix(".lwo") {
        format!("generated/model/{stem}.bmodel")
    } else if model.ends_with(".bmodel") {
        format!("generated/model/{model}")
    } else {
        return Err(format!("static model {model} is not a supported rigid source model"));
    };
    let geometry = Geometry::Rigid(Arc::new(bmodel::acquire(catalog, &resource,
        bmodel::CommonModels::shared(catalog))?.ok_or_else(||format!("static model missing: {resource}"))?));
    let descriptor = if let Some(base) = base {
        let mut descriptor = (*base).clone();
        descriptor.model = model.to_owned();
        descriptor.geometry = geometry;
        descriptor.placement.resource_path = resource;
        descriptor
    } else {
        Descriptor {entity:entity.to_owned(),class:class.to_owned(),model:model.to_owned(),
            cast_shadows:true,name_source:entity.to_owned(),geometry,
            placement:entities::PlacedModel{id:entity.to_owned(),resource_path:resource,
                position:[0.;3],rotation:[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],scale:[1.;3],
                custom_materials:Vec::new(),max_visible_range:None}}
    };
    check_active(cancelled)?;
    Ok(Arc::new(descriptor))
}

/// Weak declaration projections only. Geometry acquisition stays with the
/// canonical BML/BTRI owner; this table never pins an unselected ammunition.
#[derive(Default)]
pub(crate) struct Sources {
    catalog: Option<u64>,
    live: HashMap<String, Weak<Descriptor>>,
    upgrades: HashMap<String, Weak<Upgrade>>,
}

impl Sources {
    pub(crate) fn acquire(
        &mut self,
        catalog: &Catalog,
        entity: &str,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Arc<Descriptor>, String> {
        check_active(cancelled)?;
        if self.catalog.is_some_and(|id| id != catalog.identity()) {
            return Err("ammunition attachment belongs to another catalog".into());
        }
        self.catalog = Some(catalog.identity());
        let key = entity.replace('\\', "/").to_ascii_lowercase();
        if !catalog.has_overlays() {
            if let Some(value) = self.live.get(&key).and_then(Weak::upgrade) {
                return Ok(value);
            }
        }
        self.live.retain(|_, value| value.strong_count() != 0);
        let mut source = SourceRead {
            catalog,
            cancelled,
            texts: HashMap::new(),
        };
        let mut fields = Vec::new();
        for ancestor in entities::player::entity_chain(&key, &mut |path| source.read(path))? {
            entities::overlay_owned(&mut fields, ancestor);
        }
        let class = entities::scene_motion::atom(&fields, "class")
            .ok_or("ammunition attachment class is absent")?
            .to_owned();
        // These child constructors use idEntity. Component admission below
        // supports its actual simple AnimBasic family, rejecting other
        // component constructors before a managed identity is published.
        if class != "idEntity" {
            return Err(format!(
                "{key}: attachment class {class} has no lifecycle adapter"
            ));
        }
        let edit = entities::scene_motion::child(&fields, "edit");
        let component_list =
            entities::player::entity_components_with_prefix(&key, "", |path| source.read(path))?;
        let animation = match component_list.as_slice() {
            [] => None,
            [(kind, fields)] if kind == "cpntanimbase" => {
                let tree = entities::scene_motion::atom(fields, "m_animTree")
                    .ok_or("attachment animation tree is absent")?;
                let path = format!("generated/decls/animtree/{tree}.decl");
                let text = source
                    .read(&path)?
                    .ok_or_else(|| format!("attachment animation tree missing: {path}"))?;
                super::tree::anim_basic_only(&text)?;
                Some(tree.to_owned())
            }
            _ => {
                return Err(format!(
                    "{key}: attachment component family has no lifecycle adapter"
                ));
            }
        };
        let graph = entities::scene_motion::child(edit, "m_kiscule");
        if graph.iter().any(|(name, _)| name != "m_version") {
            return Err(format!(
                "{key}: graph-bearing ammunition attachment is unsupported"
            ));
        }
        let model = entities::scene_motion::atom(
            entities::scene_motion::child(edit, "renderModelInfo"),
            "model",
        )
        .ok_or("ammunition attachment has no render model")?
        .to_owned();
        let cast_shadows = match entities::scene_motion::atom(
            entities::scene_motion::child(edit, "renderModelInfo"),
            "m_castShadows",
        ) {
            None | Some("true") => true,
            Some("false") => false,
            Some(value) => return Err(format!("{key}: invalid m_castShadows {value}")),
        };
        // 44dd00/62b240/61a2b0 supply the real parent frame in spawn
        // parameters, so these definitions legitimately omit map placement.
        // The shared model descriptor needs a local placement scope only to
        // project scale/materials; runtime Binding supplies the actual frame.
        for (name, value) in &mut fields {
            if name == "edit" {
                if let entities::Value::Scope(edit) = value {
                    if !edit.iter().any(|(name, _)| name == "spawnPosition") {
                        edit.push(("spawnPosition".into(), entities::Value::Scope(Vec::new())));
                    }
                }
            }
        }
        let definition = entities::Definition {
            kind: "entity".into(),
            class: class.clone(),
            id: key.clone(),
            fields,
        };
        let placement = entities::model_descriptor(&definition, |path| source.read(path))?
            .ok_or("ammunition attachment model descriptor is absent")?;
        check_active(cancelled)?;
        let geometry = if model.ends_with(".lwo") && animation.is_none() {
            Geometry::Rigid(Arc::new(
                bmodel::acquire(
                    catalog,
                    &placement.resource_path,
                    bmodel::CommonModels::shared(catalog),
                )?
                .ok_or_else(|| format!("attachment model missing: {}", placement.resource_path))?,
            ))
        } else if model.ends_with(".md6") && animation.is_some() {
            let rig = Arc::new(BasicRig::load_cancellable(catalog, &model, cancelled)?);
            let mesh =
                bmd6::ModelSources::new(catalog).acquire(catalog, &placement.resource_path)?;
            Geometry::Animated { rig, model: mesh }
        } else {
            return Err(format!(
                "{key}: model/animation component pairing is unsupported: {model}"
            ));
        };
        check_active(cancelled)?;
        let descriptor = Arc::new(Descriptor {
            entity: key.clone(),
            class,
            model,
            cast_shadows,
            placement,
            geometry,
            name_source: key.clone(),
        });
        if !catalog.has_overlays() {
            self.live.insert(key, Arc::downgrade(&descriptor));
        }
        Ok(descriptor)
    }
    /// The same weak/source/model owner also projects native equipment-upgrade
    /// children. It does not acquire every upgrade or ammunition variant.
    pub(crate) fn acquire_upgrade(
        &mut self,
        catalog: &Catalog,
        declaration: &str,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Arc<Upgrade>, String> {
        check_active(cancelled)?;
        if self.catalog.is_some_and(|id| id != catalog.identity()) {
            return Err("upgrade attachment belongs to another catalog".into());
        }
        self.catalog = Some(catalog.identity());
        let key = declaration.replace('\\', "/").to_ascii_lowercase();
        if !catalog.has_overlays() {
            if let Some(value) = self.upgrades.get(&key).and_then(Weak::upgrade) {
                return Ok(value);
            }
        }
        self.upgrades.retain(|_, value| value.strong_count() != 0);
        let mut source = SourceRead {
            catalog,
            cancelled,
            texts: HashMap::new(),
        };
        let fields =
            entities::player::resolved_decl("equipmentupgrade", &key, |path| source.read(path))?;
        let info = entities::scene_motion::child(&fields, "m_upgradeInfo");
        let name = |field| {
            entities::scene_motion::atom(info, field)
                .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
        };
        let tag = name("m_attachmentTag").unwrap_or("").to_owned();
        let animation = Animation::read(name("m_attachmentAnimation"))?;
        let attachment = if let Some(model) = name("m_attachmentStaticModel") {
            // 61a2b0 creates a plain idEntity from this actual model resource,
            // disables shadows (87b6b0), then binds it. There is no entitydef
            // or fabricated component declaration for this constructor path.
            if !model.ends_with(".lwo") {
                return Err(format!(
                    "{key}: direct upgrade model is not rigid LWO: {model}"
                ));
            }
            let resource = format!(
                "generated/model/{}.bmodel",
                model.strip_suffix(".lwo").unwrap()
            );
            let geometry =
                bmodel::acquire(catalog, &resource, bmodel::CommonModels::shared(catalog))?
                    .ok_or_else(|| format!("upgrade model missing: {resource}"))?;
            Some(Arc::new(Descriptor {
                entity: key.clone(),
                class: "idEntity".into(),
                model: model.to_owned(),
                cast_shadows: false,
                name_source: "idEntity".into(),
                placement: entities::PlacedModel {
                    id: key.clone(),
                    resource_path: resource,
                    position: [0.; 3],
                    rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    scale: [1.; 3],
                    custom_materials: Vec::new(),
                    max_visible_range: None,
                },
                geometry: Geometry::Rigid(Arc::new(geometry)),
            }))
        } else {
            None
        };
        let entity = name("m_attachmentDef")
            .map(|entity| self.acquire(catalog, entity, cancelled))
            .transpose()?;
        if let Some(attachment) = entity.as_ref() {
            if let Geometry::Animated { rig, .. } = &attachment.geometry {
                if animation != Animation::Static && rig.template.items.is_empty() {
                    return Err(format!(
                        "{key}: animated upgrade has no actual AnimBasic input item zero"
                    ));
                }
            }
        }
        check_active(cancelled)?;
        let value = Arc::new(Upgrade {
            declaration: key.clone(),
            attachments: [attachment, entity],
            tag,
            animation,
            ammunition: entities::scene_motion::items(entities::scene_motion::child(&fields, "m_ammoUpgradeInfo"))
                .into_iter().map(AmmoUpgrade::read).collect::<Result<_, _>>()?,
        });
        if !catalog.has_overlays() {
            self.upgrades.insert(key, Arc::downgrade(&value));
        }
        Ok(value)
    }
}
