//!712640 declaration defaults;714560/712da0 supply the actual owner frame.
use crate::assets::entities::{machinery::{ConstraintSettings,read_constraint_fields},Value};

#[derive(Clone,Debug)]
pub(crate) struct Definition {
    settings: ConstraintSettings,
    pub disable_collision: bool,
    linear_stabilized: bool,
    breakable: bool,
}
impl Definition {
    pub(crate) fn settings(&self) -> &ConstraintSettings { &self.settings }
    pub(crate) fn read(fields:&[(String,Value)])->Result<Self,String>{
        use crate::assets::entities::components::atom;
        let flag = |key: &str, default| match atom(fields, key) {
            None => Ok(default), Some("true"|"1") => Ok(true), Some("false"|"0") => Ok(false),
            Some(value) => Err(format!("Constraint invalid {key}={value}")),
        };
        let disable_collision = flag("m_disableCollision", true)?;
        //These neutral values are declaration-local, not a body/actor pose.
        //at_frame MUST supply the actual published source owner before birth.
        let settings=read_constraint_fields(fields,"constraint declaration",[0.;3],
            [[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]])?;
        Ok(Self{settings,disable_collision,
            linear_stabilized: flag("m_linearStabilized", true)?,
            breakable: flag("m_breakable", false)?})
    }
    pub(crate) fn at_frame(&self,name:&str,frame:crate::gameplay::reference_frame::ReferenceFrame)->ConstraintSettings{
        let mut settings=self.settings.clone();
        settings.constraint_id=name.to_owned();
        settings.pivot=frame.position;
        settings.basis=frame.basis;
        settings.axis=frame.basis[settings.angular_axis];
        settings
    }
    pub(crate) fn endpoints(&self)->[Option<&str>;2]{
        [self.settings.entity_a.as_deref(),self.settings.entity_b.as_deref()]
    }
    pub(crate) fn joints(&self)->[Option<&str>;2]{
        [self.settings.joint_a.as_deref(),self.settings.joint_b.as_deref()]
    }
    /// Admission boundary of the existing factored atom rows, not a fallback
    /// conversion of a different declaration type into a hinge.
    pub(crate) fn passive_rows_supported(&self) -> bool {
        self.settings.motor.is_none()
            && self.linear_stabilized && !self.breakable
            && self.settings.linear_constraint == "LINEAR_CONSTRAINT_FIXED"
            && matches!(self.settings.angular_constraint.as_str(),
                "ANGULAR_CONSTRAINT_CONE" | "ANGULAR_CONSTRAINT_HINGE"
                | "ANGULAR_CONSTRAINT_LIMITED_HINGE")
    }
}

pub(crate) struct Binding {
    pub entity: String,
    pub slot: String,
    pub definition: std::sync::Arc<Definition>,
    pub scheduling: crate::gameplay::component_state::declaration::DeclarationScheduling,
}

/// The same component-list delta and declaration resolver used by runtime
/// definitions. Admission decides ownership after this immutable projection;
/// resolving a declaration does not construct a second legacy assembly.
pub(crate) fn bindings(
    catalog: &crate::archive::Catalog,
    document: &crate::assets::entities::Document,
    excluded: &std::collections::HashSet<String>,
) -> Result<Vec<Binding>, String> {
    use crate::assets::entities::{components, event_graph::atom, player};
    let declarations = document.definitions.iter().filter(|entry| entry.kind == "component")
        .map(|entry| (entry.id.as_str(), entry)).collect();
    let mut cache = std::collections::HashMap::new();
    let mut typed = std::collections::HashMap::<String, std::sync::Arc<Definition>>::new();
    let mut read = |path: &str| crate::assets::declaration_texts::read_common_or_catalog(catalog, path);
    let mut result = Vec::new();
    for entity in document.definitions.iter().filter(|entry| entry.kind == "entity") {
        if excluded.contains(&entity.id) { continue; }
        let mut list = components::ComponentList::default();
        if components::ComponentList::expands(&entity.fields)? {
            let parent = match crate::assets::entities::field(&entity.fields, "inherit") {
                None => None,
                Some(Value::Atom(value)) => Some(value.trim_matches('"')).filter(|name| !name.is_empty()),
                Some(value) => return Err(format!("constraint entity {} invalid parent {value:?}", entity.id)),
            };
            if let Some(parent) = parent {
                for layer in player::entity_chain(parent, &mut read)? {
                    list.apply(&layer, false, &entity.id)?;
                }
            }
        }
        list.apply(&entity.fields, false, &entity.id)?;
        for (slot, entry) in list.into_components(&entity.id)? {
            let Some(fields) = components::entry_fields(&entry, "cpntconstraint",
                &declarations, &mut cache, &mut read)? else { continue; };
            let key = if let Some(name) = atom(&entry, "mapComponentDecl") {
                format!("map:{name}")
            } else if crate::assets::entities::field(&entry, "inlineDecl").is_none() {
                format!("decl:{}", atom(&entry, "inheritedDecl").unwrap_or(""))
            } else {
                format!("{}:{slot}", entity.id)
            };
            let definition = if let Some(definition) = typed.get(&key) {
                definition.clone()
            } else {
                let definition = std::sync::Arc::new(Definition::read(&fields)
                    .map_err(|error| format!("{} component {slot}: {error}", entity.id))?);
                typed.insert(key, definition.clone());
                definition
            };
            result.push(Binding {
                entity: entity.id.clone(), slot,
                definition,
                scheduling: components::scheduling(&fields,
                    crate::gameplay::component_state::declaration::DeclarationScheduling::from_base_flags(0))?,
            });
        }
    }
    Ok(result)
}
