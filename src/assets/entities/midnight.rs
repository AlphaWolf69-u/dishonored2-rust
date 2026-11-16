//! Midnight scene decls (arkDeclMidnightScene) as the player body plays
//! them: group infos, scenes with their EMTT_ANIM clip keys and EMTT_EVENT
//! point keys, SFX keys, CONDMD6 graph nodes and the settings. Barks, motion and
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
    pub marker_values: Result<Vec<SceneMarker>, String>,
    /// md6WeightGroup_t; only MD6_WEIGHTGROUP_ANIM_EXPORT_MASK (7) occurs in
    /// the installed player scenes.
    pub body_part: u8,
    pub main_clip: bool,
    pub stretched: bool,
    pub curve: Option<FloatCurve>,
}

/// A named animation marker and its time in the Midnight clip (14018e620).
#[derive(Clone, Debug, PartialEq)]
pub struct SceneMarker {
    pub name: String,
    pub time_ms: i32,
}

/// An EMTT_EVENT EMKT_POINT key (arkMidnightKeyPropEvent).
#[derive(Clone, Debug, PartialEq)]
pub struct EventKey {
    pub time_ms: i32,
    pub event: String,
    pub class: String,
    pub properties: std::sync::Arc<[(String, super::Value)]>,
}

/// An EMTT_SFX key with arkMidnightKeyPropSfx (reflection 1415c4ff0).
#[derive(Clone, Debug, PartialEq)]
pub struct SoundKey {
    pub time_ms: i32,
    pub event: String,
    pub is_clip: bool,
    pub duration_ms: i32,
    pub stop_on_interruption: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneGroup {
    pub group: u32,
    pub specific_models: Vec<String>,
    pub anim_keys: Vec<AnimKey>,
    pub events: Vec<EventKey>,
    pub sounds: Vec<SoundKey>,
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
pub struct CondBool {
    pub uid: u32,
    pub name: String,
    pub default: bool,
    pub when_true: Next,
    pub when_false: Next,
}

/// m_conditionType of an int condition (enum order STATIC, KISCULE,
/// KISCULE_PARAMETER, RANDOM; 1407cccd0 dispatches on it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IntCondition {
    /// The instance's named int variable (1407cd3e0 initializes it to 0).
    Static,
    /// 1407cccd0 has no source for this kind: the value stays -1, which
    /// selects the default output.
    Kiscule,
    /// 140652ed0 over the instance's random state (1407cd330), drawn from
    /// the game LCG (1408ac150). The flags are m_autoDisable and m_looping.
    Random { auto_disable: bool, looping: bool },
}

/// EMNT_CONDINT (arkMidnightGraphPathNodeCondInt, ctor 1401b94c0).
#[derive(Clone, Debug, PartialEq)]
pub struct CondInt {
    pub uid: u32,
    pub name: String,
    pub condition: IntCondition,
    /// m_next: output `value` (port 0x31 + value).
    pub outputs: Vec<Next>,
    /// m_nextDefault (port 0x30): a value of -1 or at least the output count.
    pub default: Next,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Loop {
    pub uid: u32,
    pub name: String,
    pub count: i32,
    pub break_instantly: bool,
    pub break_blend: f32,
    pub looping: Next,
    pub end: Next,
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
    pub bool_nodes: Vec<CondBool>,
    pub int_nodes: Vec<CondInt>,
    pub loops: Vec<Loop>,
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

/// Decode a single anim-track key. Shared by player and world scenes.
pub(super) fn anim_key(key: &Fields) -> Result<AnimKey, String> {
    let prop = object(child(key, "m_keyProp"));
    let body_part = match atom(prop, "m_bodyPart").unwrap_or("MD6_WEIGHTGROUP_ANIM_EXPORT_MASK") {
        "MD6_WEIGHTGROUP_ANIM_EXPORT_MASK" => 7,
        other => return Err(format!("midnight body part {other} is not decoded")),
    };
    let marker_fields = child(prop, "m_lMarker");
    let markers = items(marker_fields)
        .map(|marker| Ok(SceneMarker {
            name: atom(marker, "m_name").ok_or("missing animation marker name")?.to_owned(),
            time_ms: int(marker, "m_time", 0)?,
        }))
        .collect::<Result<Vec<_>, String>>();
    Ok(AnimKey {
        start_ms: int(key, "m_startTime", 0)?,
        duration_ms: int(key, "m_duration", 0)?,
        priority: int(key, "m_priority", 0)?,
        anim: atom(prop, "m_AnimName").unwrap_or("").to_owned(),
        start_offset: int(prop, "m_startOffset", 0)?,
        end_offset: int(prop, "m_endOffset", 0)?,
        markers: int(marker_fields, "num", 0)? as usize,
        marker_values: markers,
        body_part,
        main_clip: boolean(prop, "m_isMainClip"),
        stretched: boolean(prop, "m_isStretched"),
        curve: curve(key)?,
    })
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
            let mut specific_models = atoms(child(g, "m_specificMD6s")).into_iter().map(str::to_owned).collect::<Vec<_>>();
            if let Some(model) = atom(g, "m_specificMD6").filter(|model| !model.is_empty() && *model != "NULL") {
                specific_models.push(model.to_owned());
            }
            let mut group = SceneGroup {
                group: uid(child(g, "m_handle"), "groupUID")?, specific_models,
                anim_keys: Vec::new(), events: Vec::new(), sounds: Vec::new(),
            };
            for track in items(child(g, "m_lTracks")).map(object) {
                let track_type = atom(track, "m_trackType");
                for key in items(child(track, "m_keys")) {
                    let prop = object(child(key, "m_keyProp"));
                    if atom(key, "m_playingDirection").is_some_and(|d| d != "EMPD_BOTH" && d != "EMPD_FORWARD") {
                        continue;
                    }
                    match track_type {
                        Some("EMTT_ANIM") => {
                            group.anim_keys.push(anim_key(key)?);
                        }
                        Some("EMTT_EVENT") => group.events.push(EventKey {
                            time_ms: int(key, "m_startTime", 0)?,
                            event: atom(prop, "m_event").unwrap_or("").to_owned(),
                            class: atom(child(key, "m_keyProp"), "m_class").unwrap_or("").to_owned(),
                            properties: prop.to_vec().into(),
                        }),
                        Some("EMTT_SFX") => group.sounds.push(SoundKey {
                            time_ms: int(key, "m_startTime", 0)?,
                            event: atom(prop, "m_soundEvent").unwrap_or("").to_owned(),
                            is_clip: atom(key, "m_type") == Some("EMKT_CLIP"),
                            duration_ms: int(key, "m_duration", 0)?,
                            stop_on_interruption: boolean(prop, "m_stopEventOnInterruption"),
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
    let mut bool_nodes = Vec::new();
    let mut int_nodes = Vec::new();
    let mut loops = Vec::new();
    for wrapper in items(child(edit, "m_nodes")) {
        let node = object(wrapper);
        let properties = object(child(node, "m_properties"));
        let id = uid(child(node, "m_uid"), "m_objUID")?;
        match atom(node, "m_nodeType") {
            Some("EMNT_CONDBOOL") => {
                // 1b9130 defaults STATIC and true; 7cd3e0 initializes the
                // per-instance value by the inherited node name.
                if atom(properties, "m_conditionType").is_some_and(|kind| kind != "EMNCT_STATIC") {
                    return Err("Midnight external bool condition provider is not admitted".into());
                }
                bool_nodes.push(CondBool { uid: id, name: atom(properties, "m_nodeName").unwrap_or("").to_owned(),
                    default: atom(properties, "m_defaultPath").map_or(true, |value| value == "true"),
                    when_true: next(child(node, "m_nextTrue"))?, when_false: next(child(node, "m_nextFalse"))? });
                continue;
            }
            Some("EMNT_CONDINT") => {
                let condition = match atom(properties, "m_conditionType").unwrap_or("EMNCT_STATIC") {
                    "EMNCT_STATIC" => IntCondition::Static,
                    "EMNCT_KISCULE" => IntCondition::Kiscule,
                    "EMNCT_RANDOM" => IntCondition::Random {
                        auto_disable: boolean(properties, "m_autoDisable"),
                        looping: boolean(properties, "m_looping"),
                    },
                    // 1407cccd0 reads the owning script entity's parameter.
                    "EMNCT_KISCULE_PARAMETER" => return Err("Midnight int script-parameter condition is not admitted".into()),
                    other => return Err(format!("Midnight int condition {other} is not decoded")),
                };
                let outputs = items(child(node, "m_next")).map(next).collect::<Result<Vec<_>, _>>()?;
                // 1401b9de0 sizes the output list by m_outputCount.
                if outputs.len() != int(properties, "m_outputCount", 1)? as usize {
                    return Err("Midnight int condition output count differs from its outputs".into());
                }
                int_nodes.push(CondInt { uid: id, name: atom(properties, "m_nodeName").unwrap_or("").to_owned(),
                    condition, outputs, default: next(child(node, "m_nextDefault"))? });
                continue;
            }
            Some("EMNT_LOOP") => {
                // 1b9fc0: infinite count, deferred break, .3s break blend.
                loops.push(Loop { uid: id, name: atom(properties, "m_nodeName").unwrap_or("").to_owned(),
                    count: int(properties, "m_nbLoop", -1)?, break_instantly: boolean(properties, "m_breakInstantly"),
                    break_blend: float(properties, "m_breakInstantBlendTime", 0.3).unwrap_or(0.3),
                    looping: next(child(node, "m_nextLooping"))?, end: next(child(node, "m_nextEndLoop"))? });
                continue;
            }
            _ => {}
        }
        if atom(node, "m_nodeType") != Some("EMNT_CONDMD6") {
            return Err(format!("midnight node {:?} is not decoded", atom(node, "m_nodeType")));
        }
        let mut choices = Vec::new();
        for choice in items(child(node, "m_next")) {
            choices.push((atom(choice, "m_md6").unwrap_or("").to_owned(), next(child(choice, "m_nextInfo"))?));
        }
        nodes.push(CondMd6 {
            uid: id,
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
        bool_nodes,
        int_nodes,
        loops,
        first: next(child(edit, "m_firstElement"))?,
    })
}
