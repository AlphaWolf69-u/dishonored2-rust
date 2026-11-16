//! Speech declaration owner: 140189d10/140189d50/14018a1d0 and
//! 1404e6b80. Immutable metadata only; Wwise owns encoded/streamed media.
use crate::assets::{archive::Catalog, entities::Value};
use crate::gameplay::options::decl::{atom, list_items, scope, unquote};
use std::sync::Arc;

#[derive(Debug)]
enum Kind {
    Barks {
        hash: String,
        name: String,
        roles: Vec<String>,
        lines: Vec<(u32, usize, bool)>,
    },
    Character(Vec<String>),
    Voice(String),
}

#[derive(Debug)]
pub struct Definition {
    kind: Kind,
    source_paths: Vec<String>,
}

#[derive(Default)]
pub(crate) struct Resources {
    barks: crate::assets::resource_registry::Registry<Definition>,
    characters: crate::assets::resource_registry::Registry<Definition>,
    voices: crate::assets::resource_registry::Registry<Definition>,
}
impl Resources {
    fn registry(
        &mut self,
        kind: &str,
    ) -> &mut crate::assets::resource_registry::Registry<Definition> {
        match kind.to_ascii_lowercase().as_str() {
            "speechbarks" => &mut self.barks,
            "speechcharacter" => &mut self.characters,
            "speechvoice" => &mut self.voices,
            _ => unreachable!(),
        }
    }
}
pub struct Recording {
    pub events: Vec<u32>,
    // Keep precisely the acquired declaration generations until playback ends.
    _definitions: Vec<Arc<Definition>>,
}

pub(crate) fn is_kind(kind: &str) -> bool {
    ["speechbarks", "speechcharacter", "speechvoice"]
        .iter()
        .any(|name| kind.eq_ignore_ascii_case(name))
}

fn path(kind: &str, name: &str, language: &str) -> String {
    let prefix = if kind == "speechbarks" {
        format!("generated/decls/localized/{language}")
    } else {
        "generated/decls".to_owned()
    };
    format!(
        "{prefix}/{kind}/{}.decl",
        name.replace('\\', "/").to_ascii_lowercase()
    )
}

fn acquire(
    catalog: &Catalog,
    kind: &str,
    name: &str,
    language: &str,
    marker: u8,
) -> Result<Arc<Definition>, String> {
    let path = path(kind, name, language);
    catalog
        .speech_resources
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "speech declaration owner poisoned")?
        .registry(kind)
        .acquire_with_sharing(&path, marker, !catalog.has_overlays(), |_| {
            // Reuse the declaration overlay resolver, mapping localized
            // speechBarks reads into the selected audio-language namespace.
            let mut source_paths = Vec::new();
            let fields = crate::assets::entities::player::resolved_decl(kind, name, |resource| {
                let resource = if kind == "speechbarks" {
                    resource.replacen(
                        "generated/decls/",
                        &format!("generated/decls/localized/{language}/"),
                        1,
                    )
                } else {
                    resource.to_owned()
                };
                source_paths.push(resource.replace('\\', "/").to_ascii_lowercase());
                catalog
                    .read(&resource)
                    .map_err(|error| error.to_string())
                    .map(|bytes| bytes.map(crate::assets::entities::owned_declaration_text))
            })?;
            let edit = fields.as_slice();
            let strings = |key| {
                scope(edit, key)
                    .into_iter()
                    .flat_map(list_items)
                    .filter_map(|value| {
                        if let Value::Atom(value) = value {
                            Some(unquote(value).to_owned())
                        } else {
                            None
                        }
                    })
                    .collect()
            };
            let kind = match kind {
                "speechcharacter" => Kind::Character(strings("m_voiceList")),
                "speechvoice" => Kind::Voice(atom(edit, "m_voiceName").unwrap_or("").to_owned()),
                "speechbarks" => {
                    let roles = scope(edit, "m_speechRoles")
                        .into_iter()
                        .flat_map(list_items)
                        .map(|role| {
                            if let Value::Scope(role) = role {
                                atom(role, "m_character").unwrap_or("").to_owned()
                            } else {
                                String::new()
                            }
                        })
                        .collect();
                    let values = scope(edit, "m_speechLines")
                        .into_iter()
                        .flat_map(list_items);
                    // 189d50 starts missing IDs after the largest explicit ID.
                    let mut lines = Vec::new();
                    for value in values {
                        let Value::Scope(line) = value else {
                            return Err("speech line is not a scope".into());
                        };
                        let id = atom(line, "m_ID")
                            .unwrap_or("0")
                            .parse::<u32>()
                            .map_err(|_| "invalid speech line ID")?;
                        let role = atom(line, "m_roleIndex")
                            .unwrap_or("0")
                            .parse::<usize>()
                            .map_err(|_| "invalid speech role")?;
                        let corvo = scope(line, "m_corvoText")
                            .and_then(|fields| scope(fields, "m_lines"))
                            .is_some_and(|lines| !list_items(lines).is_empty());
                        lines.push((id, role, corvo));
                    }
                    let mut next = lines
                        .iter()
                        .map(|line| line.0)
                        .max()
                        .unwrap_or(0)
                        .checked_add(1)
                        .ok_or("speech ID overflow")?;
                    for line in &mut lines {
                        if line.0 == 0 {
                            line.0 = next;
                        }
                        next = next.checked_add(1).ok_or("speech ID overflow")?;
                    }
                    Kind::Barks {
                        hash: atom(edit, "m_hash").unwrap_or("").to_owned(),
                        name: name.rsplit('/').next().unwrap_or(name).to_owned(),
                        roles,
                        lines,
                    }
                }
                _ => return Err(format!("unsupported speech class {kind}")),
            };
            Ok(Arc::new(Definition { kind, source_paths }))
        })
}

fn scope_marker(catalog: &Catalog, kind: &str, name: &str, language: &str) -> Result<u8, String> {
    Ok(catalog
        .speech_resources
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "speech owner poisoned")?
        .registry(kind)
        .producer_scope(&path(kind, name, language)))
}

/// The audiocard player's 4961c0 requests each ordered line with its declared
/// role. 4e6b80 builds hash[2..9] + '_' + voiceName + '_' + basename_ID.
pub fn recording(catalog: &Catalog, card: &str, corvo: bool) -> Result<Recording, String> {
    let language = crate::assets::lang::read_audio_language(catalog.base())?;
    let barks = acquire(catalog, "speechbarks", card, language, 0)?;
    let Kind::Barks {
        hash,
        name,
        roles,
        lines,
    } = &barks.kind
    else {
        unreachable!()
    };
    let hash = hash
        .get(2..9)
        .ok_or("speech hash must contain its seven-character event prefix")?;
    let marker = scope_marker(catalog, "speechbarks", card, language)?;
    let mut definitions = Vec::new();
    let mut voices = Vec::new();
    for role in roles {
        let character = acquire(catalog, "speechcharacter", role, language, marker)?;
        let Kind::Character(list) = &character.kind else {
            unreachable!()
        };
        // Audiograph roles must resolve unambiguously; never pick an arbitrary
        // NPC voice from a multi-voice speech character.
        if list.len() != 1 {
            return Err(format!(
                "audiograph role {role} requires its live voice binding ({})",
                list.len()
            ));
        }
        let voice = acquire(
            catalog,
            "speechvoice",
            &list[0],
            language,
            scope_marker(catalog, "speechcharacter", role, language)?,
        )?;
        let Kind::Voice(voice_name) = &voice.kind else {
            unreachable!()
        };
        voices.push(voice_name.clone());
        definitions.extend([character, voice]);
    }
    let events = lines
        .iter()
        .map(|&(id, role, has_corvo)| {
            let voice = voices
                .get(role)
                .ok_or("speech line role outside declared role list")?;
            Ok(crate::assets::wwise::event_id(&format!(
                "{hash}_{voice}_{name}_{id}{}",
                if corvo && has_corvo { "_c" } else { "" }
            )))
        })
        .collect::<Result<Vec<_>, String>>()?;
    definitions.push(barks);
    Ok(Recording {
        events,
        _definitions: definitions,
    })
}

///The same speechBarks resource generation referenced by an actual Bark
///outcome and CoordinationBB record. No media is decoded by this acquisition.
pub fn acquire_barks(catalog:&Catalog,path:&str)->Result<Arc<Definition>,String> {
    let language=crate::assets::lang::read_audio_language(catalog.base())?;
    acquire(catalog,"speechbarks",path,language,0)
}
pub fn acquire_voice(catalog:&Catalog,path:&str)->Result<Arc<Definition>,String>{
    let language=crate::assets::lang::read_audio_language(catalog.base())?;
    acquire(catalog,"speechvoice",path,language,0)
}
///75c... Speech owns both declarations through the actual Wwise End receipt.
///The Device prepares its voice header once; selecting a line does no I/O.
pub fn selected_line_with_voice(set:Arc<Definition>,line:u32,voice:Arc<Definition>,corvo:bool)->Result<Recording,String>{
    let Kind::Barks {hash,name,lines,..}=&set.kind else {return Err("Bark line resource has a different actual speech class".into())};
    let &(_,_,has_corvo)=lines.iter().find(|entry|entry.0==line).ok_or_else(||format!("speech line {line} absent in {name}"))?;
    let Kind::Voice(voice_name)=&voice.kind else {return Err("Bark component voice resource class differs".into())};
    let hash=hash.get(2..9).ok_or("speech hash must contain its seven-character event prefix")?;
    let event=crate::assets::wwise::event_id(&format!("{hash}_{voice_name}_{name}_{line}{}",if corvo&&has_corvo {"_c"} else {""}));
    Ok(Recording {events:vec![event],_definitions:vec![set,voice]})
}

pub(crate) fn register_preload(
    catalog: &Catalog,
    kind: &str,
    path: &str,
    marker: u8,
    persistent: bool,
) -> Result<(), String> {
    catalog
        .speech_resources
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "speech owner poisoned")?
        .registry(kind)
        .register(&path.to_ascii_lowercase(), marker, persistent);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.speech_resources.get() {
        let mut owner = owner.lock().map_err(|_| "speech owner poisoned")?;
        for kind in ["speechbarks", "speechcharacter", "speechvoice"] {
            owner.registry(kind).promote_bootstrap();
        }
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.speech_resources.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        for kind in ["speechBarks", "speechCharacter", "speechVoice"] {
            owner
                .registry(kind)
                .release(catalog.identity(), kind, marker);
        }
        let dependencies = owner
            .barks
            .live_values()
            .chain(owner.characters.live_values())
            .chain(owner.voices.live_values())
            .flat_map(|definition| definition.source_paths.clone())
            .collect::<std::collections::HashSet<_>>();
        for kind in ["speechBarks", "speechCharacter", "speechVoice"] {
            let registry = owner.registry(kind);
            crate::loading::retain_catalog_kind(catalog.identity(), kind, |path| {
                registry.owns_read_history(path) || dependencies.contains(path)
            });
        }
    }
}
