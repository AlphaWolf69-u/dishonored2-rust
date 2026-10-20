//! Authored motion-only scene loops and the entity graphs that start them.
use super::{field, float, parse_declaration, scope, vector, Document, Value};
use crate::archive::Catalog;
use std::collections::HashMap;

pub type Fields = Vec<(String, Value)>;
#[derive(Clone, Copy, Debug)]
pub enum Interpolation {
    Constant,
    Linear,
    CatmullRom,
    Bezier,
}
#[derive(Clone, Debug)]
pub struct CurveKey {
    pub time_ms: i32,
    pub position: [f32; 3],
    pub orientation: [f32; 3],
    pub tangent_in: [f32; 3],
    pub tangent_out: [f32; 3],
    pub angle_in: [f32; 3],
    pub angle_out: [f32; 3],
}
#[derive(Clone, Debug)]
pub struct MotionCurve {
    pub interpolation: Interpolation,
    pub keys: Vec<CurveKey>,
}
pub fn atom<'a>(f: &'a [(String, Value)], k: &str) -> Option<&'a str> {
    match field(f, k)? {
        Value::Atom(s) => Some(s.trim_matches('"')),
        _ => None,
    }
}
pub fn child<'a>(f: &'a [(String, Value)], k: &str) -> &'a [(String, Value)] {
    scope(field(f, k)).unwrap_or(&[])
}
pub fn items(f: &[(String, Value)]) -> impl Iterator<Item = &[(String, Value)]> {
    f.iter().filter_map(|(k, v)| {
        if k.starts_with("item[") {
            scope(Some(v))
        } else {
            None
        }
    })
}
fn object(f: &[(String, Value)]) -> &[(String, Value)] {
    child(f, "m_object")
}
fn number(f: &[(String, Value)], k: &str, d: f32) -> Result<f32, String> {
    float(f, k, d)
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("invalid {k}"))
}
fn angles(f: &[(String, Value)]) -> Result<[f32; 3], String> {
    Ok([
        number(f, "pitch", 0.)?,
        number(f, "yaw", 0.)?,
        number(f, "roll", 0.)?,
    ])
}
#[derive(Clone, Debug)]
pub struct MotionGroup {
    pub uid: u32,
    pub name: String,
    pub curve: MotionCurve,
}
#[derive(Clone, Debug)]
pub struct MotionClip {
    pub length_ms: i32,
    pub groups: Vec<MotionGroup>,
}

/// Accept a whole supported graph, rather than treating any scene containing a
/// motion key as an autonomous animation. Other cinematic resources stay owned
/// by their future scene runtime. No filename or entity-name exceptions.
pub fn parse_clip(text: &str) -> Result<MotionClip, String> {
    let fields = parse_declaration(text)?;
    let edit = child(&fields, "edit");
    let scenes: Vec<_> = items(child(edit, "m_scenes")).collect();
    if scenes.len() != 1 {
        return Err("requires one scene".into());
    }
    let scene = object(scenes[0]);
    let uid = atom(child(scene, "m_objUID"), "m_objUID").unwrap_or("0");
    let first = child(edit, "m_firstElement");
    if atom(first, "m_nextType") != Some("EAMNT_SCENE")
        || atom(child(first, "m_nextCPoint"), "sceneUID") != Some(uid)
    {
        return Err("first element does not enter the motion scene".into());
    }
    let next = child(scene, "m_nextElement");
    if atom(next, "m_nextType") != Some("EAMNT_NODE") {
        return Err("scene is not a graph loop".into());
    }
    let node_uid = atom(child(next, "m_nextCPoint"), "nodeUID");
    let loop_node = items(child(edit, "m_nodes"))
        .map(object)
        .find(|n| atom(child(n, "m_uid"), "m_objUID") == node_uid)
        .ok_or("missing loop node")?;
    let props = object(child(loop_node, "m_properties"));
    let looping = child(loop_node, "m_nextLooping");
    if atom(loop_node, "m_nodeType") != Some("EMNT_LOOP")
        || atom(props, "m_nbLoop") != Some("-1")
        || atom(looping, "m_nextType") != Some("EAMNT_SCENE")
        || atom(child(looping, "m_nextCPoint"), "sceneUID") != Some(uid)
    {
        return Err("requires an infinite self loop".into());
    }
    let length = number(child(scene, "m_properties"), "m_length", 0.)? as i32;
    if length <= 0 {
        return Err("invalid scene length".into());
    }
    let infos: Vec<_> = items(child(edit, "m_groupsInfo")).collect();
    let mut groups = Vec::new();
    for wrapper in items(child(scene, "m_groups")) {
        let group = object(wrapper);
        if atom(group, "m_isDisabled") == Some("true") {
            continue;
        }
        let id = atom(child(group, "m_handle"), "groupUID").ok_or("missing group UID")?;
        let info = infos
            .iter()
            .copied()
            .find(|i| atom(child(i, "m_UID"), "m_objUID") == Some(id))
            .ok_or("missing group info")?;
        // Constructor8b5290: local motion axis=1, clamp loop=0, Bezier=4.
        if atom(info, "m_motionAxis").is_some() {
            return Err("explicit motion axis needs decoding".into());
        }
        if atom(info, "m_motionLoopMode").is_some() {
            return Err("explicit curve loop mode needs decoding".into());
        }
        let interpolation =
            match atom(info, "m_motionInterpolationMode").unwrap_or("ECI_BEZIER_TANGENT") {
                "ECI_CONST" => Interpolation::Constant,
                "ECI_LINEAR" => Interpolation::Linear,
                "ECI_CATMULLROMSPLINE" => Interpolation::CatmullRom,
                "ECI_BEZIER_TANGENT" => Interpolation::Bezier,
                other => return Err(format!("unsupported interpolation {other}")),
            };
        let tracks: Vec<_> = items(child(group, "m_lTracks")).map(object).collect();
        if tracks.len() != 1 || atom(tracks[0], "m_trackType") != Some("EMTT_MOTION") {
            return Err("scene has non-motion tracks".into());
        }
        let mut keys = Vec::new();
        for key in items(child(tracks[0], "m_keys")) {
            if atom(key, "m_type") != Some("EMKT_POINT") {
                return Err("non-point motion key".into());
            }
            let prop = child(key, "m_keyProp");
            if atom(prop, "m_class") != Some("arkMidnightKeyPropMotion") {
                return Err("non-motion property".into());
            }
            let p = object(prop);
            let pos =
                |name| vector(child(p, name), [0.; 3]).ok_or_else(|| format!("invalid {name}"));
            keys.push(CurveKey {
                time_ms: number(key, "m_startTime", 0.)? as i32,
                position: pos("m_position")?,
                orientation: angles(child(p, "m_orientation"))?,
                tangent_in: pos("m_tangentIn")?,
                tangent_out: pos("m_tangentOut")?,
                angle_in: angles(child(p, "m_tangentInOr"))?,
                angle_out: angles(child(p, "m_tangentOutOr"))?,
            });
        }
        if keys.is_empty() || keys.windows(2).any(|p| p[0].time_ms >= p[1].time_ms) {
            return Err("invalid motion key times".into());
        }
        groups.push(MotionGroup {
            uid: id.parse().map_err(|_| "invalid group UID")?,
            name: atom(info, "m_name").unwrap_or("").into(),
            curve: MotionCurve {
                interpolation,
                keys,
            },
        });
    }
    if groups.is_empty() {
        return Err("no motion groups".into());
    }
    Ok(MotionClip {
        length_ms: length,
        groups,
    })
}

#[derive(Clone, Debug)]
pub struct Port {
    pub id: u32,
    pub name: String,
    pub kind: String,
    pub links: Vec<(u32, u32)>,
}
#[derive(Clone, Debug)]
pub struct Node {
    pub id: u32,
    pub class: String,
    pub params: Fields,
    pub ports: Vec<Port>,
}
#[derive(Clone, Debug)]
pub struct Graph {
    pub owner: String,
    pub nodes: Vec<Node>,
    pub clips: HashMap<String, MotionClip>,
}
pub fn load(catalog: &Catalog, document: &Document) -> Result<Vec<Graph>, String> {
    let mut cache = HashMap::new();
    let mut result = Vec::new();
    for entity in document.definitions.iter().filter(|d| d.kind == "entity") {
        let list = child(child(child(&entity.fields, "edit"), "m_kiscule"), "m_nodes");
        if !items(list).any(|n| {
            matches!(
                atom(n, "m_name"),
                Some(
                    "arkKisculeNodeActionMidnightKeyframeControl"
                        | "arkKisculeNodeActionAngularPhysicsMotorControl"
                )
            )
        }) {
            continue;
        }
        let mut nodes = Vec::new();
        let mut clips = HashMap::new();
        for n in items(list) {
            let params = object(child(n, "m_parameters")).to_vec();
            if let Some(path) = atom(&params, "m_declMdnt").filter(|p| *p != "NULL") {
                if !cache.contains_key(path) {
                    let resource = format!("generated/decls/midnightscene/{path}.decl");
                    let clip = catalog
                        .read(&resource)
                        .map_err(|e| e.to_string())?
                        .and_then(|b| parse_clip(&super::declaration_text(&b)).ok());
                    cache.insert(path.to_owned(), clip);
                }
                if let Some(clip) = &cache[path] {
                    clips.insert(path.to_owned(), clip.clone());
                }
            }
            let mut ports = Vec::new();
            for p in items(child(n, "m_connectionPoints")) {
                let links = items(child(p, "m_connections"))
                    .filter_map(|c| {
                        Some((
                            atom(c, "m_endNodeID")?.parse().ok()?,
                            atom(c, "m_endConnectionPointID")?.parse().ok()?,
                        ))
                    })
                    .collect();
                ports.push(Port {
                    id: atom(p, "m_id")
                        .and_then(|s| s.parse().ok())
                        .ok_or("invalid port ID")?,
                    name: atom(p, "m_name").unwrap_or("").into(),
                    kind: atom(p, "m_type").unwrap_or("").into(),
                    links,
                });
            }
            nodes.push(Node {
                id: atom(n, "m_id")
                    .and_then(|s| s.parse().ok())
                    .ok_or("invalid node ID")?,
                class: atom(n, "m_name").unwrap_or("").into(),
                params,
                ports,
            });
        }
        if !clips.is_empty()
            || nodes
                .iter()
                .any(|n| n.class == "arkKisculeNodeActionAngularPhysicsMotorControl")
        {
            result.push(Graph {
                owner: entity.id.clone(),
                nodes,
                clips,
            });
        }
    }
    Ok(result)
}
