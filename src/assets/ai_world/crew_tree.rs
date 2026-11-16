//! Crew's executable arktree projection. The shared gameflow/arktree registry
//! owns source residency; consumers lease typed data, never generic syntax.
use crate::{
    archive::Catalog,
    entities::Value,
    gameplay::{
        gameflow::program::Projection,
        options::decl::{atom, list_items, read_edit, scope},
    },
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Kind {
    Loop(i32),
    Parallel {
        first_fail: bool,
        first_success: bool,
    },
    Idle,
    DebugLog {
        output_to_console: bool,
        return_immediately: bool,
        display_time: f32,
    },
}
#[derive(Debug)]
pub(crate) struct Node {
    pub kind: Kind,
    pub children: Box<[usize]>,
}
#[derive(Debug)]
pub(crate) struct Data {
    pub path: Arc<str>,
    pub nodes: Box<[Node]>,
    pub parallel_count: usize,
    pub maximum_depth: usize,
}
pub struct Definition {
    _resource: Arc<Projection>,
    data: Arc<Data>,
}
impl std::fmt::Debug for Definition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CrewTree")
            .field("path", &self.data.path)
            .field("nodes", &self.data.nodes.len())
            .finish()
    }
}
impl Definition {
    ///588310 obtains8acd20's arktree resource before5d6fd0 instantiation.
    ///The existing registry handles map markers, overlay generations and purge.
    pub fn acquire(catalog: &Catalog, path: &str) -> Result<Arc<Self>, String> {
        let name = path.to_ascii_lowercase();
        let source = format!("generated/decls/arktree/{name}.decl");
        let resource = catalog
            .gameflow_programs
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "arktree resource owner poisoned")?
            .acquire_with_sharing(&source, 0, !catalog.has_overlays(), |_| {
                let mut failure = None;
                let mut read = |path: &str| match catalog.read(path) {
                    Ok(value) => value,
                    Err(error) => {
                        failure = Some(error.to_string());
                        None
                    }
                };
                let fields = read_edit(&mut read, "arktree", &name);
                if let Some(error) = failure {
                    return Err(error);
                }
                let fields = fields?;
                let root =
                    scope(&fields, "root").ok_or_else(|| format!("arktree {name} has no root"))?;
                let mut data = Data {
                    path: source.clone().into(),
                    nodes: Box::default(),
                    parallel_count: 0,
                    maximum_depth: 0,
                };
                let mut nodes = Vec::new();
                project(root, &mut nodes, &mut data, 1)?;
                data.nodes = nodes.into_boxed_slice();
                Ok(Arc::new(Projection::Crew(Arc::new(data))))
            })?;
        let data = match &*resource {
            Projection::Crew(data) => data.clone(),
            Projection::Gameflow(_) => {
                return Err(format!("arktree {name} has a gameflow projection"));
            }
        };
        Ok(Arc::new(Self {
            _resource: resource,
            data,
        }))
    }
    pub(crate) fn data(&self) -> &Data {
        &self.data
    }
}
fn boolean(fields: &[(String, Value)], name: &str, default: bool) -> Result<bool, String> {
    match atom(fields, name) {
        None => Ok(default),
        Some("true" | "1") => Ok(true),
        Some("false" | "0") => Ok(false),
        Some(value) => Err(format!("invalid arktree {name}: {value}")),
    }
}
fn project(
    fields: &[(String, Value)],
    nodes: &mut Vec<Node>,
    data: &mut Data,
    depth: usize,
) -> Result<usize, String> {
    let settings = scope(fields, "m_settings").ok_or("arktree node settings absent")?;
    let class = atom(settings, "m_class").ok_or("arktree node class absent")?;
    let object = scope(settings, "m_object").unwrap_or(&[]);
    if scope(object, "m_stimSettings").is_some_and(|stim| {
        ["m_handled", "m_consumed"]
            .iter()
            .any(|name| atom(stim, name).is_some_and(|value| !value.is_empty()))
    }) {
        return Err(format!(
            "arktree node {class} requires unimplemented stimulus dispatch"
        ));
    }
    let kind = match class {
        "arkTreeNodeSettings_Loop" => Kind::Loop(
            atom(object, "m_stopCondition")
                .unwrap_or("-1")
                .parse()
                .map_err(|_| "invalid arktree loop stop condition")?,
        ),
        "arkTreeNodeSettings_Parallel" => {
            data.parallel_count += 1;
            Kind::Parallel {
                first_fail: boolean(object, "m_returnOnFirstFail", false)?,
                first_success: boolean(object, "m_returnOnFirstSuccess", false)?,
            }
        }
        "arkTreeNodeSettings_Action_Idle" => Kind::Idle,
        //932a60 defaults;5d3130 consumes these three settings. The release
        //task never reads its debug text/colour/scale, so they need no copy.
        "arkTreeNodeSettings_Action_DebugLog" => Kind::DebugLog {
            output_to_console: boolean(object, "m_outputToConsole", false)?,
            return_immediately: boolean(object, "m_returnImmediately", false)?,
            display_time: atom(object, "m_displayTime")
                .unwrap_or("0")
                .parse()
                .map_err(|_| "invalid arktree debug display time")?,
        },
        _ => return Err(format!("unsupported executable Crew arktree node {class}")),
    };
    let index = nodes.len();
    nodes.push(Node {
        kind,
        children: Box::default(),
    });
    data.maximum_depth = data.maximum_depth.max(depth);
    let mut children = Vec::new();
    for value in scope(fields, "m_children")
        .map(list_items)
        .unwrap_or_default()
    {
        let Value::Scope(child) = value else {
            return Err("arktree child is not a node".into());
        };
        children.push(project(child, nodes, data, depth + 1)?);
    }
    let count = children.len();
    if match kind {
        Kind::Loop(_) => count != 1,
        Kind::Parallel { .. } => count != 2,
        _ => count != 0,
    } {
        return Err(format!(
            "arktree node {class} has invalid child count {count}"
        ));
    }
    nodes[index].children = children.into_boxed_slice();
    Ok(index)
}
