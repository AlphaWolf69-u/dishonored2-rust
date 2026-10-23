//! Three-way merge of Cargo manifests by key, for mods that change
//! `Cargo.toml`: two mods adding different dependencies, features or
//! binaries merge even when their lines touch, and a change only conflicts
//! when both sides changed the same key differently.
//!
//! - A key changed on one side only takes that side (removal included).
//! - Tables (inline or not) merge key by key.
//! - Arrays (feature lists, `[[bin]]`) merge as sets: both sides' additions
//!   are kept, either side's removals are applied.
//! - A dependency both sides changed merges field by field; its `features`
//!   merge as a set, and two caret-compatible versions (`1.2` and `1.4`)
//!   take the higher, which satisfies both.
use std::collections::BTreeMap;
use toml_edit::{Array, DocumentMut, InlineTable, Item, TableLike, Value};

/// A value with formatting removed, for comparing the three versions.
#[derive(Clone, Debug, PartialEq)]
enum Canon {
    Scalar(String),
    List(Vec<Canon>),
    Map(BTreeMap<String, Canon>),
}

fn value_canon(v: &Value) -> Canon {
    match v {
        Value::String(s) => Canon::Scalar(format!("s:{}", s.value())),
        Value::Array(a) => Canon::List(a.iter().map(value_canon).collect()),
        Value::InlineTable(t) => Canon::Map(t.iter().map(|(k, v)| (k.to_string(), value_canon(v))).collect()),
        other => {
            let mut v = other.clone();
            v.decor_mut().clear();
            Canon::Scalar(v.to_string())
        }
    }
}

fn canon(item: Option<&Item>) -> Option<Canon> {
    match item? {
        Item::None => None,
        Item::Value(v) => Some(value_canon(v)),
        Item::Table(t) => Some(Canon::Map(t.iter().filter_map(|(k, i)| Some((k.to_string(), canon(Some(i))?))).collect())),
        Item::ArrayOfTables(a) => Some(Canon::List(a.iter().filter_map(|t| canon(Some(&Item::Table(t.clone())))).collect())),
    }
}

/// Merge `theirs` into `ours`, both made from `base`. Returns the merged
/// manifest, or the keys both sides changed differently.
pub fn merge(base: &str, ours: &str, theirs: &str) -> Result<String, String> {
    let parse = |which: &str, text: &str| text.parse::<DocumentMut>().map_err(|e| format!("{which} is not valid TOML: {e}"));
    let base = parse("the common version", base)?;
    let mut out = parse("the merged mods' version", ours)?;
    let theirs = parse("the mod's version", theirs)?;
    let mut conflicts = Vec::new();
    merge_table(&[], Some(base.as_table()), out.as_table_mut(), theirs.as_table(), &mut conflicts);
    if conflicts.is_empty() {
        // toml_edit writes LF; keep the merged side's line endings.
        let text = out.to_string();
        Ok(if ours.contains("\r\n") { text.replace("\r\n", "\n").replace('\n', "\r\n") } else { text })
    } else {
        Err(conflicts.join(", "))
    }
}

fn merge_table(path: &[String], base: Option<&dyn TableLike>, ours: &mut dyn TableLike, theirs: &dyn TableLike, conflicts: &mut Vec<String>) {
    let mut keys: Vec<String> = ours.iter().map(|(k, _)| k.to_string()).collect();
    for (k, _) in theirs.iter() {
        if !keys.iter().any(|o| o == k) {
            keys.push(k.to_string());
        }
    }
    for key in keys {
        let b = base.and_then(|t| t.get(&key));
        let t = theirs.get(&key);
        let (cb, co, ct) = (canon(b), canon(ours.get(&key)), canon(t));
        if co == ct || cb == ct {
            continue;
        }
        if cb == co {
            match t {
                Some(t) => {
                    ours.insert(&key, t.clone());
                }
                None => {
                    ours.remove(&key);
                }
            }
            continue;
        }
        let mut here = path.to_vec();
        here.push(key.clone());
        let (Some(o), Some(t)) = (ours.get_mut(&key), t) else {
            conflicts.push(format!("{} (removed on one side, changed on the other)", here.join(".")));
            continue;
        };
        if is_dependency(path) {
            match merge_dependency(&here, b, o, t) {
                Ok(merged) => *o = merged,
                Err(e) => conflicts.push(e),
            }
        } else if o.is_table_like() && t.is_table_like() {
            let b = b.and_then(Item::as_table_like);
            merge_table(&here, b, o.as_table_like_mut().unwrap(), t.as_table_like().unwrap(), conflicts);
        } else if let (Some(oa), Some(ta)) = (o.as_array_mut(), t.as_array()) {
            merge_array(b.and_then(Item::as_array), oa, ta);
        } else if let (Some(oa), Some(ta)) = (o.as_array_of_tables_mut(), t.as_array_of_tables()) {
            let item = |t: &toml_edit::Table| canon(Some(&Item::Table(t.clone())));
            let based: Vec<_> = b.and_then(Item::as_array_of_tables).map(|a| a.iter().map(item).collect()).unwrap_or_default();
            let kept: Vec<_> = ta.iter().map(item).collect();
            oa.retain(|e| !based.contains(&item(e)) || kept.contains(&item(e)));
            let present: Vec<_> = oa.iter().map(item).collect();
            for e in ta.iter().filter(|e| !based.contains(&item(e)) && !present.contains(&item(e))) {
                oa.push(e.clone());
            }
        } else {
            conflicts.push(here.join("."));
        }
    }
}

/// Set merge: `ours` without what `theirs` removed, plus what it added.
fn merge_array(base: Option<&Array>, ours: &mut Array, theirs: &Array) {
    let based: Vec<Canon> = base.map(|a| a.iter().map(value_canon).collect()).unwrap_or_default();
    let kept: Vec<Canon> = theirs.iter().map(value_canon).collect();
    ours.retain(|v| !based.contains(&value_canon(v)) || kept.contains(&value_canon(v)));
    let present: Vec<Canon> = ours.iter().map(value_canon).collect();
    for v in theirs.iter().filter(|v| !based.contains(&value_canon(v)) && !present.contains(&value_canon(v))) {
        ours.push(v.clone());
    }
}

/// `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]` (also
/// under `[target.*]` and `[workspace]`) and `[patch.<registry>]`.
fn is_dependency(path: &[String]) -> bool {
    matches!(path.last().map(String::as_str), Some("dependencies" | "dev-dependencies" | "build-dependencies")) || (path.len() == 2 && path[0] == "patch")
}

/// A dependency's fields: `"1.2"` is `{ version = "1.2" }`.
fn dependency_fields(item: Option<&Item>) -> BTreeMap<String, Value> {
    match item {
        Some(Item::Value(Value::String(s))) => BTreeMap::from([("version".to_string(), Value::from(s.value().as_str()))]),
        Some(i) => i.as_table_like().map(|t| t.iter().filter_map(|(k, i)| Some((k.to_string(), i.as_value()?.clone()))).collect()).unwrap_or_default(),
        None => BTreeMap::new(),
    }
}

fn merge_dependency(here: &[String], base: Option<&Item>, ours: &Item, theirs: &Item) -> Result<Item, String> {
    let (b, o, t) = (dependency_fields(base), dependency_fields(Some(ours)), dependency_fields(Some(theirs)));
    let name = here.join(".");
    let mut out = o.clone();
    let keys: Vec<&String> = o.keys().chain(t.keys().filter(|k| !o.contains_key(*k))).collect();
    for key in keys {
        let (cb, co, ct) = (b.get(key).map(value_canon), o.get(key).map(value_canon), t.get(key).map(value_canon));
        if co == ct || cb == ct {
            continue;
        }
        if cb == co {
            match t.get(key) {
                Some(v) => out.insert(key.clone(), v.clone()),
                None => out.remove(key),
            };
            continue;
        }
        match (key.as_str(), o.get(key), t.get(key)) {
            ("version", Some(ov), Some(tv)) => {
                let (ov, tv) = (ov.as_str().unwrap_or(""), tv.as_str().unwrap_or(""));
                let v = higher_compatible(ov, tv).ok_or_else(|| format!("{name} (version {ov:?} and {tv:?} are not compatible)"))?;
                out.insert(key.clone(), Value::from(v));
            }
            ("features", Some(Value::Array(oa)), Some(Value::Array(ta))) => {
                let mut merged = oa.clone();
                merge_array(b.get(key).and_then(Value::as_array), &mut merged, ta);
                out.insert(key.clone(), Value::Array(merged));
            }
            _ => return Err(format!("{name}.{key}")),
        }
    }
    if out.len() == 1 && out.contains_key("version") {
        let mut v = out.remove("version").unwrap();
        v.decor_mut().clear();
        return Ok(Item::Value(v));
    }
    let mut table = InlineTable::new();
    for (k, v) in out.iter().filter(|(k, _)| *k == "version").chain(out.iter().filter(|(k, _)| *k != "version")) {
        let mut v = v.clone();
        v.decor_mut().clear();
        table.insert(k, v);
    }
    Ok(Item::Value(Value::InlineTable(table)))
}

/// Of two caret requirements (`1.2`, `^1.4.0`) in the same compatibility
/// range, the higher, which satisfies both; `None` for other requirements.
fn higher_compatible<'a>(a: &'a str, b: &'a str) -> Option<&'a str> {
    let parse = |r: &str| -> Option<Vec<u64>> {
        let r = r.trim().trim_start_matches('^');
        let parts: Vec<u64> = r.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
        (1..=3).contains(&parts.len()).then_some(parts)
    };
    // Cargo's caret rule: the left-most non-zero component must match.
    let range = |v: &[u64]| -> Vec<u64> {
        let first = v.iter().position(|&n| n != 0).unwrap_or(v.len() - 1);
        v[..=first].to_vec()
    };
    let (pa, pb) = (parse(a)?, parse(b)?);
    let pad = |v: &[u64]| [0, 1, 2].map(|i| v.get(i).copied().unwrap_or(0));
    if range(&pad(&pa)) != range(&pad(&pb)) {
        return None;
    }
    Some(if pad(&pa) >= pad(&pb) { a } else { b })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"[package]
name = "dishonored2-rust"
version = "0.1.0"

[features]
game = ["dep:bevy"]

[dependencies]
anyhow = "1.0"
bevy = { version = "0.18", optional = true, default-features = false, features = ["png"] }

[[bin]]
name = "dishonored2-rust"
path = "src/bin/game.rs"
"#;

    #[test]
    fn dependencies_added_next_to_each_other_merge() {
        let ours = BASE.replace("anyhow = \"1.0\"\n", "anyhow = \"1.0\"\nrand = \"0.9\"\n");
        let theirs = BASE.replace("anyhow = \"1.0\"\n", "anyhow = \"1.0\"\nserde = \"1\"\n");
        let merged: DocumentMut = merge(BASE, &ours, &theirs).unwrap().parse().unwrap();
        assert_eq!(merged["dependencies"]["rand"].as_str(), Some("0.9"));
        assert_eq!(merged["dependencies"]["serde"].as_str(), Some("1"));
        assert_eq!(merged["dependencies"]["anyhow"].as_str(), Some("1.0"));
    }

    #[test]
    fn features_lists_and_binaries_merge_as_sets() {
        let ours = BASE.replace("game = [\"dep:bevy\"]", "game = [\"dep:bevy\", \"dep:rand\"]\nextra = []")
            + "\n[[bin]]\nname = \"a\"\npath = \"src/bin/a.rs\"\n";
        let theirs = BASE.replace("game = [\"dep:bevy\"]", "game = [\"dep:serde\", \"dep:bevy\"]")
            + "\n[[bin]]\nname = \"b\"\npath = \"src/bin/b.rs\"\n";
        let merged: DocumentMut = merge(BASE, &ours, &theirs).unwrap().parse().unwrap();
        let game: Vec<_> = merged["features"]["game"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        assert_eq!(game, ["dep:bevy", "dep:rand", "dep:serde"]);
        assert!(merged["features"].get("extra").is_some());
        let bins: Vec<_> = merged["bin"].as_array_of_tables().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(bins, ["dishonored2-rust", "a", "b"]);
    }

    #[test]
    fn one_dependency_changed_by_both_merges_its_fields() {
        let ours = BASE.replace("features = [\"png\"]", "features = [\"png\", \"jpeg\"]").replace("anyhow = \"1.0\"", "anyhow = \"1.0.80\"");
        let theirs = BASE.replace("features = [\"png\"]", "features = [\"png\", \"ktx2\"]").replace("anyhow = \"1.0\"", "anyhow = { version = \"1.0.90\", features = [\"backtrace\"] }");
        let merged: DocumentMut = merge(BASE, &ours, &theirs).unwrap().parse().unwrap();
        let bevy = merged["dependencies"]["bevy"].as_inline_table().unwrap();
        let features: Vec<_> = bevy["features"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        assert_eq!(features, ["png", "jpeg", "ktx2"]);
        assert_eq!(bevy["optional"].as_bool(), Some(true));
        let anyhow = merged["dependencies"]["anyhow"].as_inline_table().unwrap();
        assert_eq!(anyhow["version"].as_str(), Some("1.0.90"));
        assert_eq!(anyhow["features"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_removal_on_one_side_applies() {
        let theirs = BASE.replace("anyhow = \"1.0\"\n", "");
        let ours = BASE.replace("[features]", "[features]\nextra = []");
        let merged: DocumentMut = merge(BASE, &ours, &theirs).unwrap().parse().unwrap();
        assert!(merged["dependencies"].get("anyhow").is_none());
        assert!(merged["features"].get("extra").is_some());
    }

    #[test]
    fn incompatible_changes_name_the_key() {
        let ours = BASE.replace("anyhow = \"1.0\"", "anyhow = \"1.0\"\nrand = \"0.8\"");
        let theirs = BASE.replace("anyhow = \"1.0\"", "anyhow = \"1.0\"\nrand = \"0.9\"");
        assert_eq!(merge(BASE, &ours, &theirs).unwrap_err(), "dependencies.rand (version \"0.8\" and \"0.9\" are not compatible)");
        let ours = BASE.replace("version = \"0.1.0\"", "version = \"0.2.0\"");
        let theirs = BASE.replace("version = \"0.1.0\"", "version = \"0.3.0\"");
        assert_eq!(merge(BASE, &ours, &theirs).unwrap_err(), "package.version");
    }

    #[test]
    fn the_rewrite_manifest_merges_two_mods() {
        // As git stores it (the checkout may have CRLF).
        let base = &include_str!("../../../Cargo.toml").replace("\r\n", "\n");
        let add = |text: &str, line: &str| text.replacen("[dependencies]\n", &format!("[dependencies]\n{line}\n"), 1);
        let (ours, theirs) = (add(base, "rand = \"0.9\""), add(base, "serde = \"1\""));
        assert_eq!(merge(base, base, &theirs).unwrap(), theirs);
        let merged = merge(base, &ours, &theirs).unwrap();
        let doc: DocumentMut = merged.parse().unwrap();
        assert!(doc["dependencies"].get("rand").is_some() && doc["dependencies"].get("serde").is_some());
        assert_eq!(merged.len(), base.len() + "rand = \"0.9\"\nserde = \"1\"\n".len());
    }

    #[test]
    fn caret_compatibility_follows_cargo() {
        assert_eq!(higher_compatible("1.2", "^1.4.1"), Some("^1.4.1"));
        assert_eq!(higher_compatible("0.18", "0.18.3"), Some("0.18.3"));
        assert_eq!(higher_compatible("0.18", "0.19"), None);
        assert_eq!(higher_compatible("0.0.3", "0.0.4"), None);
        assert_eq!(higher_compatible("=1.2", "1.3"), None);
    }
}
