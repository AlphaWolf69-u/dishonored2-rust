//! World movie declarations: FlashSurface4b0790/1416f44e0 and Map1416f5a40.
use crate::assets::entities::Value;
use crate::gameplay::options::decl::{atom, boolean, float, list_items, scope, unquote};

#[derive(Debug)]
pub struct Definition {
    pub iggy_info: String,
    pub is_static_texture: bool,
    pub capture_delay: f32,
}
impl Definition {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        Ok(Self {
            iggy_info: atom(fields, "m_iggyInfo")
                .filter(|s| !s.is_empty() && *s != "NULL")
                .unwrap_or("")
                .to_owned(),
            is_static_texture: boolean(fields, "m_isStaticTexture", false),
            capture_delay: float(fields, "m_delayCaptureStaticTexture", 1.),
        })
    }
}

#[derive(Debug)]
pub struct MapDefinition {
    pub title: String,
    pub legends: Vec<String>,
    pub has_player_position: bool,
    pub theme: i32,
}
impl MapDefinition {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let theme = match atom(fields, "m_type") {
            None | Some("ARK_UI_INTERACTIVE_MAP_TYPE_NEUTRAL") | Some("0") => 0,
            Some(other) => return Err(format!("unknown interactive map theme {other}")),
        };
        Ok(Self {
            title: atom(fields, "m_title").unwrap_or("").to_owned(),
            legends: scope(fields, "m_legends")
                .map(|list| {
                    list_items(list)
                        .into_iter()
                        .map(|value| match value {
                            Value::Atom(value) => Ok(unquote(value).to_owned()),
                            _ => Err("interactive map legend is not an idStrId".to_owned()),
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default(),
            has_player_position: boolean(fields, "m_mapHasPlayerPosition", false),
            theme,
        })
    }
}
