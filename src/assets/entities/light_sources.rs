//! Temporary entityDef source ownership for light preparation.
//! Registration141352590; identity1400607f0; parent-before-child140022fd0.
use super::{Value, light_atom, parse_declaration};
use std::{collections::HashMap, sync::Arc};
type Fields = Vec<(String, Value)>;
pub(super) struct Source {
    parent: Option<Arc<Source>>,
    edits: Vec<Fields>,
    height: usize,
}
impl Source {
    pub(super) fn append<'a>(&'a self, layers: &mut Vec<&'a [(String, Value)]>) {
        if let Some(parent) = &self.parent {
            parent.append(layers);
        }
        layers.extend(self.edits.iter().map(Vec::as_slice));
    }
}
#[derive(Default)]
pub(super) struct Sources {
    entries: HashMap<String, Arc<Source>>,
}
impl Sources {
    pub(super) fn acquire(
        &mut self,
        name: &str,
        depth: usize,
        read: &mut impl FnMut(&str) -> Result<Option<String>, String>,
    ) -> Result<Arc<Source>, String> {
        if depth > 16 {
            return Err(format!(
                "light declaration inheritance depth exceeds 16 at {name}"
            ));
        }
        let path = format!("generated/decls/entitydef/{name}.decl");
        let key = path.replace('\\', "/").to_ascii_lowercase();
        if let Some(source) = self.entries.get(&key) {
            // Preserve the previous total inheritance-depth validation even
            // when an ancestor has already been decoded for another light.
            if depth + source.height <= 16 {
                return Ok(source.clone());
            }
        }
        let _tag = crate::platform::allocation_tags::tag("Light declaration sources");
        let text = read(&path)?
            .ok_or_else(|| format!("unresolved light declaration inheritance: {path}"))?;
        let mut fields = parse_declaration(&text).map_err(|error| format!("{path}: {error}"))?;
        let parent = light_atom(&fields, "inherit")
            .map(str::to_owned)
            .map(|parent| self.acquire(&parent, depth + 1, read))
            .transpose()?;
        // Preserve both source views and their old precedence; move only the
        // edits needed by this consumer, dropping unrelated parser fields.
        let mut edits = Vec::new();
        if let Some((_, Value::Scope(edit))) = fields.iter_mut().find(|(key, _)| key == "edit") {
            edits.push(std::mem::take(edit));
        }
        if let Some(edit) = fields.iter_mut().find_map(|(key, value)| {
            if !key.starts_with("\"idLight\"") {
                return None;
            }
            if let Value::Scope(edit) = value {
                Some(edit)
            } else {
                None
            }
        }) {
            edits.push(std::mem::take(edit));
        }
        let height = parent.as_ref().map_or(0, |parent| parent.height + 1);
        let source = Arc::new(Source {
            parent,
            edits,
            height,
        });
        self.entries.insert(key, source.clone());
        Ok(source)
    }
    pub(super) fn retire(self, trace: bool) {
        if !trace {
            return;
        }
        let leases: Vec<_> = self.entries.values().map(Arc::downgrade).collect();
        let count = leases.len();
        drop(self);
        let live = leases
            .iter()
            .filter(|lease| lease.strong_count() != 0)
            .count();
        eprintln!("LIGHT_SOURCE_RETIRE declarations={count} live={live}");
    }
}
