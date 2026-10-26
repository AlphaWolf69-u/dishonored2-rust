//! Level audio components, retaining map-local declarations, inherited edits,
//! null event slots and component scheduling.
use super::{components::{atom, component_fields, entry_fields, scheduling}, field, scope, Document};
use crate::{assets::wwise::SoundEventDeclaration, gameplay::{component_state::declaration::DeclarationScheduling,
    sound::component::{Settings, Startup}}};
use std::collections::HashMap;

pub struct AudioBinding {
    pub entity_id: String,
    pub slot: String,
    pub scheduling: DeclarationScheduling,
    pub settings: Settings,
    pub propagation: bool,
    pub events: Vec<Option<SoundEventDeclaration>>,
}
impl AudioBinding {
    pub fn startup(&self) -> Startup { Startup::from_events(&self.events) }
}

pub fn audio_bindings(document: &Document,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Vec<AudioBinding>, String>
{
    let declarations = document.definitions.iter().filter(|definition| definition.kind == "component")
        .map(|definition| (definition.id.as_str(), definition)).collect::<HashMap<_, _>>();
    let mut cache = HashMap::new();
    let mut event_fields = HashMap::new();
    let mut events = HashMap::<String, SoundEventDeclaration>::new();
    let mut result = Vec::new();
    for entity in document.definitions.iter().filter(|definition| definition.kind == "entity") {
        let Some(entries) = scope(field(&entity.fields, "edit")).and_then(|edit| scope(field(edit, "m_componentDecls"))) else { continue };
        for (slot, entry) in entries.iter().filter(|(key, _)| key.starts_with("item_add[")) {
            let Some(entry) = scope(Some(entry)) else { continue };
            let Some(fields) = entry_fields(entry, "cpntaudio", &declarations, &mut cache, &mut read)? else { continue };
            let list = scope(field(&fields, "m_startSoundEvents")).unwrap_or(&[]);
            let count = atom(list, "num").unwrap_or("0").parse::<usize>()
                .map_err(|_| format!("{}: invalid startup sound count", entity.id))?;
            let mut start_events = Vec::with_capacity(count);
            for index in 0..count {
                let name = atom(list, &format!("item[{index}]"))
                    .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL"));
                let event = match name {
                    None => None,
                    Some(name) => {
                        if !events.contains_key(name) {
                            let fields = component_fields("soundevent", name, 0, &mut event_fields, &mut read)?;
                            let event = SoundEventDeclaration::from_fields(&fields).map_err(|error| format!("{name}: {error}"))?;
                            events.insert(name.to_owned(), event);
                        }
                        Some(events[name].clone())
                    }
                };
                start_events.push(event);
            }
            let propagation = match atom(&fields, "m_enablePropagation") {
                None | Some("true") => true,
                Some("false") => false,
                _ => return Err(format!("{}: invalid audio propagation flag", entity.id)),
            };
            let mut defaults = DeclarationScheduling::from_base_flags(0);
            defaults.add_updates(0x10); // 14000ce50
            result.push(AudioBinding { entity_id: entity.id.clone(), slot: slot.clone(),
                scheduling: scheduling(&fields, defaults)?, settings: Settings::read(&fields)?,
                propagation, events: start_events });
        }
    }
    Ok(result)
}
