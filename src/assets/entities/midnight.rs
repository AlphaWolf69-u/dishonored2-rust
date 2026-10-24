//! Midnight scene decls (arkDeclMidnightScene) as the player body plays
//! them: group infos, scenes with their EMTT_ANIM clip keys and EMTT_EVENT
//! point keys, CONDMD6 graph nodes and the settings. Sfx, barks, motion and
//! timed-event tracks are not read here.
use super::scene_motion::{atom, child, items};
use super::{field, float, parse_declaration, Value};

/// arkMidnightNextType: an element of the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    Final,
    Scene(u32),
    Node(u32),
}

/// arkMidnightKeyCurve_Float: `m_times` (ms) and `m_floatValues` (value,
/// tangent in, tangent out per key) under `m_interpMode`.
#[derive(Clone, Debug, PartialEq)]
pub struct FloatCurve {
    /// curveInterpolation_t: 1 CONST, 2 LINEAR, 3 CATMULLROMSPLINE, 4 BEZIER_TANGENT.
    pub mode: u8,
    pub times: Vec<i32>,
    pub values: Vec<f32>,
}

/// An EMTT_ANIM EMKT_CLIP key with its arkMidnightKeyPropAnim.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimKey {
    pub start_ms: i32,
    pub duration_ms: i32,
    pub priority: i32,
    pub anim: String,
    pub start_offset: i32,
    pub end_offset: i32,
    pub markers: usize,
    /// md6WeightGroup_t; only MD6_WEIGHTGROUP_ANIM_EXPORT_MASK (7) occurs in
    /// the installed player scenes.
    pub body_part: u8,
    pub main_clip: bool,
    pub stretched: bool,
    pub curve: Option<FloatCurve>,
}

/// An EMTT_EVENT EMKT_POINT key (arkMidnightKeyPropEvent).
#[derive(Clone, Debug, PartialEq)]
pub struct EventKey {
    pub time_ms: i32,
    pub event: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneGroup {
    pub group: u32,
    pub anim_keys: Vec<AnimKey>,
    pub events: Vec<EventKey>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub uid: u32,
    pub length_ms: i32,
    pub groups: Vec<SceneGroup>,
    pub next: Next,
}

/// EMNT_CONDMD6 (arkMidnightGraphPathNodeCondMD6).
#[derive(Clone, Debug, PartialEq)]
pub struct CondMd6 {
    pub uid: u32,
    pub group: String,
    pub next: Vec<(String, Next)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MidnightScene {
    /// m_settings (ctor FUN_140193e90): m_blendOutTime 0.3, m_blendInOverrideTime -1.
    pub blend_out_time: f32,
    pub blend_in_override_time: f32,
    /// (uid, m_name).
    pub groups: Vec<(u32, String)>,
    pub scenes: Vec<Scene>,
    pub nodes: Vec<CondMd6>,
    pub first: Next,
}

impl MidnightScene {
    pub fn group_name(&self, uid: u32) -> Option<&str> {
        self.groups.iter().find(|(id, _)| *id == uid).map(|(_, name)| name.as_str())
    }
}

type Fields = [(String, Value)];

fn object(f: &Fields) -> &Fields {
    child(f, "m_object")
}

fn int(f: &Fields, key: &str, default: i32) -> Result<i32, String> {
    match atom(f, key) {
        None => Ok(default),
        Some(text) => text.parse::<f64>().map(|v| v as i32).map_err(|_| format!("invalid {key} {text}")),
    }
}

fn boolean(f: &Fields, key: &str) -> bool {
    atom(f, key) == Some("true")
}

fn uid(f: &Fields, key: &str) -> Result<u32, String> {
    atom(f, key).and_then(|v| v.parse().ok()).ok_or_else(|| format!("missing {key}"))
}

/// The atom list of an `item[n] = value` scope, in index order.
fn atoms<'a>(f: &'a Fields) -> Vec<&'a str> {
    let mut entries: Vec<(usize, &str)> = f
        .iter()
        .filter_map(|(k, v)| match v {
            Value::Atom(text) => Some((k.strip_prefix("item[")?.strip_suffix(']')?.parse().ok()?, text.trim_matches('"'))),
            Value::Scope(_) => None,
        })
        .collect();
    entries.sort();
    entries.into_iter().map(|(_, v)| v).collect()
}

fn next(f: &Fields) -> Result<Next, String> {
    let point = child(f, "m_nextCPoint");
    Ok(match atom(f, "m_nextType") {
        Some("EAMNT_FINAL") | None => Next::Final,
        Some("EAMNT_SCENE") => Next::Scene(uid(point, "sceneUID")?),
        Some("EAMNT_NODE") => Next::Node(uid(point, "nodeUID")?),
        Some(other) => return Err(format!("unsupported midnight next type {other}")),
    })
}

fn curve(key: &Fields) -> Result<Option<FloatCurve>, String> {
    let Some(Value::Scope(curves)) = field(key, "m_keyCurves") else { return Ok(None) };
    if atom(curves, "m_class") != Some("arkMidnightKeyCurve_Float") {
        return Err("unsupported midnight key curve".into());
    }
    let c = object(curves);
    let mode = match atom(c, "m_interpMode").unwrap_or("ECI_BEZIER_TANGENT") {
        "ECI_CONST" => 1,
        "ECI_LINEAR" => 2,
        "ECI_CATMULLROMSPLINE" => 3,
        "ECI_BEZIER_TANGENT" => 4,
        other => return Err(format!("unsupported curve interpolation {other}")),
    };
    let times = atoms(child(c, "m_times")).into_iter().map(|t| t.parse::<f64>().map(|v| v as i32)).collect::<Result<Vec<_>, _>>();
    let values = atoms(child(c, "m_floatValues")).into_iter().map(str::parse::<f32>).collect::<Result<Vec<_>, _>>();
    match (times, values) {
        (Ok(times), Ok(values)) => Ok(Some(FloatCurve { mode, times, values })),
        _ => Err("invalid midnight float curve".into()),
    }
}

pub fn parse(text: &str) -> Result<MidnightScene, String> {
    let fields = parse_declaration(text)?;
    let edit = child(&fields, "edit");
    let settings = child(edit, "m_settings");
    let mut groups = Vec::new();
    for info in items(child(edit, "m_groupsInfo")) {
        groups.push((uid(child(info, "m_UID"), "m_objUID")?, atom(info, "m_name").unwrap_or("").to_owned()));
    }
    let mut scenes = Vec::new();
    for wrapper in items(child(edit, "m_scenes")) {
        let scene = object(wrapper);
        let mut scene_groups = Vec::new();
        for g in items(child(scene, "m_groups")).map(object) {
            if boolean(g, "m_isDisabled") {
                continue;
            }
            let mut group = SceneGroup { group: uid(child(g, "m_handle"), "groupUID")?, anim_keys: Vec::new(), events: Vec::new() };
            for track in items(child(g, "m_lTracks")).map(object) {
                let track_type = atom(track, "m_trackType");
                for key in items(child(track, "m_keys")) {
                    let prop = object(child(key, "m_keyProp"));
                    if atom(key, "m_playingDirection").is_some_and(|d| d != "EMPD_BOTH" && d != "EMPD_FORWARD") {
                        continue;
                    }
                    match track_type {
                        Some("EMTT_ANIM") => {
                            let body_part = match atom(prop, "m_bodyPart").unwrap_or("MD6_WEIGHTGROUP_ANIM_EXPORT_MASK") {
                                "MD6_WEIGHTGROUP_ANIM_EXPORT_MASK" => 7,
                                other => return Err(format!("midnight body part {other} is not decoded")),
                            };
                            group.anim_keys.push(AnimKey {
                                start_ms: int(key, "m_startTime", 0)?,
                                duration_ms: int(key, "m_duration", 0)?,
                                priority: int(key, "m_priority", 0)?,
                                anim: atom(prop, "m_AnimName").unwrap_or("").to_owned(),
                                start_offset: int(prop, "m_startOffset", 0)?,
                                end_offset: int(prop, "m_endOffset", 0)?,
                                markers: int(child(prop, "m_lMarker"), "num", 0)? as usize,
                                body_part,
                                main_clip: boolean(prop, "m_isMainClip"),
                                stretched: boolean(prop, "m_isStretched"),
                                curve: curve(key)?,
                            });
                        }
                        Some("EMTT_EVENT") => group.events.push(EventKey {
                            time_ms: int(key, "m_startTime", 0)?,
                            event: atom(prop, "m_event").unwrap_or("").to_owned(),
                        }),
                        _ => {}
                    }
                }
            }
            scene_groups.push(group);
        }
        scenes.push(Scene {
            uid: uid(child(scene, "m_objUID"), "m_objUID")?,
            length_ms: int(child(scene, "m_properties"), "m_length", 0)?,
            groups: scene_groups,
            next: next(child(scene, "m_nextElement"))?,
        });
    }
    let mut nodes = Vec::new();
    for wrapper in items(child(edit, "m_nodes")) {
        let node = object(wrapper);
        if atom(node, "m_nodeType") != Some("EMNT_CONDMD6") {
            return Err(format!("midnight node {:?} is not decoded", atom(node, "m_nodeType")));
        }
        let mut choices = Vec::new();
        for choice in items(child(node, "m_next")) {
            choices.push((atom(choice, "m_md6").unwrap_or("").to_owned(), next(child(choice, "m_nextInfo"))?));
        }
        nodes.push(CondMd6 {
            uid: uid(child(node, "m_uid"), "m_objUID")?,
            group: atom(object(child(node, "m_properties")), "m_groupName").unwrap_or("").to_owned(),
            next: choices,
        });
    }
    Ok(MidnightScene {
        blend_out_time: float(settings, "m_blendOutTime", 0.3).unwrap_or(0.3),
        blend_in_override_time: float(settings, "m_blendInOverrideTime", -1.0).unwrap_or(-1.0),
        groups,
        scenes,
        nodes,
        first: next(child(edit, "m_firstElement"))?,
    })
}
