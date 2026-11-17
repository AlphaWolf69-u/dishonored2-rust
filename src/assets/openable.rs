//! arkDeclCpntOpenable:74a670 and reflected1416.../143c0bdc0..0c630.
//! The component owns these compact fields; child models, clips, damage and
//! audio keep their existing declaration/resource acquisition owners.
use crate::{assets::entities::{Value,event_graph::{atom,child,entity_reference_names}},
    gameplay::interaction_definition::InteractionDef};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(crate) enum InitialState { Opened, OpenedCounterClockwise, Closed }

#[derive(Debug)]
pub(crate) struct Definition {
    pub linked:Option<String>,
    pub animated_part:String,
    pub attachment_tag:String,
    pub peeking_tag:String,
    pub front_tag:String,
    pub back_tag:String,
    pub handle_tag:String,
    pub usages:[InteractionDef;5],
    pub usage_labels:[[String;2];5],
    ///OpenCW/CCW,CloseCW/CCW,LockedCW/CCW,WindblastCW/CCW.
    pub animations:[String;8],
    pub allow_keyhole:bool,
    pub usable_by_player:bool,
    pub initial:InitialState,
    pub locked:bool,
    pub needed_keys:Vec<std::sync::Arc<str>>,
    pub blockers:Vec<String>,
    pub low_opened_damage:Option<String>,
    pub high_opened_damage:Option<String>,
    pub immune_to_windblast:bool,
    pub fov_fx:Option<String>,
    pub unlocked_sound:Option<String>,
    pub unlocked_event:Option<std::sync::Arc<crate::assets::wwise::SoundEventDeclaration>>,
}
fn reference(fields:&[(String,Value)],key:&str)->Option<String>{
    atom(fields,key).filter(|value|!value.is_empty()&&!value.eq_ignore_ascii_case("NULL")).map(str::to_owned)
}
fn flag(fields:&[(String,Value)],key:&str,default:bool)->Result<bool,String>{
    match atom(fields,key){None=>Ok(default),Some("true"|"1")=>Ok(true),Some("false"|"0")=>Ok(false),
        Some(value)=>Err(format!("Openable {key}: invalid boolean {value}"))}
}
impl Definition {
    pub(crate) fn read(catalog:&crate::archive::Catalog,fields:&[(String,Value)])->Result<Self,String>{
        let initial=match atom(fields,"m_defaultState"){
            None|Some("ARK_DOOR_DEFAULT_CLOSED"|"2")=>InitialState::Closed,
            Some("ARK_DOOR_DEFAULT_OPENED"|"0")=>InitialState::Opened,
            Some("ARK_DOOR_DEFAULT_OPENED_CCW"|"1")=>InitialState::OpenedCounterClockwise,
            Some(value)=>return Err(format!("Openable m_defaultState: {value}")),
        };
        let mut needed_keys=Vec::new();
        for (key,value) in child(fields,"m_neededKeys"){
            if !key.starts_with("item["){continue;}
            let Value::Atom(value)=value else{return Err("Openable key is not a declaration reference".into())};
            let value=value.trim_matches('"');
            if !value.is_empty()&&!value.eq_ignore_ascii_case("NULL"){needed_keys.push(std::sync::Arc::from(value));}
        }
        let usage_names=["m_usageOpen","m_usageClose","m_usageLocked","m_usageBlocked","m_usageKeyhole"];
        let usages=usage_names.map(|name|InteractionDef::from_fields(child(fields,name)))
            .into_iter().collect::<Result<Vec<_>,_>>()?.try_into().map_err(|_|"Openable interaction count")?;
        let animation_names=["m_animOpenCW","m_animOpenCCW","m_animCloseCW","m_animCloseCCW",
            "m_animLockedCW","m_animLockedCCW","m_animWindblastCW","m_animWindblastCCW"];
        let unlocked_sound=reference(fields,"m_unlockedSoundEvent");
        let unlocked_event=unlocked_sound.as_deref().map(|source|crate::assets::wwise::event_resources::acquire_with(catalog,source,
            &mut |path: &str|crate::assets::declaration_texts::read_common_or_catalog(catalog,path))).transpose()?;
        Ok(Self{
            linked:reference(fields,"m_linkedOpenableEntity"),
            animated_part:reference(fields,"m_animatedPartDef").ok_or("Openable actual animated-part definition absent")?,
            attachment_tag:atom(fields,"m_attachementTag").unwrap_or("").to_owned(),
            peeking_tag:atom(fields,"m_peekingTag").unwrap_or("").to_owned(),
            front_tag:atom(fields,"m_frontTag").unwrap_or("").to_owned(),
            back_tag:atom(fields,"m_backTag").unwrap_or("").to_owned(),
            handle_tag:atom(fields,"m_handleTag").unwrap_or("").to_owned(),
            usages,usage_labels:usage_names.map(|name|[atom(child(fields,name),"m_name").unwrap_or("").to_owned(),
                atom(child(fields,name),"m_usageText").unwrap_or("").to_owned()]),
            animations:animation_names.map(|name|atom(fields,name).unwrap_or("").to_owned()),
            allow_keyhole:flag(fields,"m_allowKeyhole",true)?,usable_by_player:flag(fields,"m_usableByPlayer",true)?,
            initial,locked:flag(fields,"m_locked",false)?,
            needed_keys,blockers:entity_reference_names(fields,"m_blockingEntities")?.into_iter().flatten().collect(),
            low_opened_damage:reference(fields,"m_lowOpenedDamage"),high_opened_damage:reference(fields,"m_highOpenedDamage"),
            immune_to_windblast:flag(fields,"m_immuneToWindblast",false)?,
            fov_fx:reference(fields,"m_FOV_Fx"),unlocked_sound,unlocked_event,
        })
    }
}

pub(crate) struct Binding {
    pub entity:String,
    pub definition:std::sync::Arc<Definition>,
    pub scheduling:crate::gameplay::component_state::declaration::DeclarationScheduling,
}
pub(crate) fn bindings(catalog:&crate::archive::Catalog,document:&crate::assets::entities::Document)
    ->Result<Vec<Binding>,String>{
    use crate::assets::entities::{components,scope};
    let declarations=document.definitions.iter().filter(|entry|entry.kind=="component")
        .map(|entry|(entry.id.as_str(),entry)).collect();
    let mut cache=std::collections::HashMap::new();
    let mut typed=std::collections::HashMap::<String,std::sync::Arc<Definition>>::new();
    let mut result=Vec::new();
    let mut read=|path:&str|crate::assets::declaration_texts::read_common_or_catalog(catalog,path);
    for entity in document.definitions.iter().filter(|entry|entry.kind=="entity"){
        for (slot,value) in child(child(&entity.fields,"edit"),"m_componentDecls"){
            if !slot.starts_with("item_add["){continue;}
            let Some(entry)=scope(Some(value)) else{continue;};
            let Some(fields)=components::entry_fields(entry,"cpntopenable",&declarations,&mut cache,&mut read)? else{continue;};
            let key=if let Some(name)=atom(entry,"mapComponentDecl"){format!("map:{name}")}
                else if crate::assets::entities::field(entry,"inlineDecl").is_none(){format!("decl:{}",atom(entry,"inheritedDecl").unwrap_or(""))}
                else{format!("{}:{slot}",entity.id)};
            let definition=if let Some(definition)=typed.get(&key){definition.clone()}else{
                let definition=std::sync::Arc::new(Definition::read(catalog,&fields).map_err(|error|format!("{} {slot}: {error}",entity.id))?);
                typed.insert(key,definition.clone());definition
            };
            result.push(Binding{entity:entity.id.clone(),definition,
                scheduling:components::scheduling(&fields,crate::gameplay::component_state::declaration::DeclarationScheduling::from_base_flags(0))?});
        }
    }
    Ok(result)
}
