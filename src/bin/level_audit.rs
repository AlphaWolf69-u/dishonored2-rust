//! Compare installed levels by authored entity/component diversity.
#[path = "../archive.rs"]
mod archive;
#[path = "../entities.rs"]
mod entities;

use std::collections::{BTreeMap, HashSet};
use std::{env, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base = env::args()
        .nth(1)
        .or_else(|| env::var("DH2_GAME_BASE").ok())
        .unwrap_or_else(|| r"D:\Programs\Steam\steamapps\common\Dishonored2\base".into());
    let catalog = archive::Catalog::open(Path::new(&base))?;
    let mut levels = Vec::new();
    let mut class_totals = BTreeMap::<String, usize>::new();
    for path in catalog.paths_with_suffix(".entities") {
        let Some(bytes) = catalog.read(&path)? else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        let document = entities::parse(&text).map_err(|error| format!("{path}: {error}"))?;
        let mut classes = HashSet::<String>::new();
        let mut interactive = 0usize;
        let mut components = HashSet::<String>::new();
        let mut entities = 0usize;
        for definition in &document.definitions {
            if definition.kind == "entity" {
                entities += 1;
                if let Some(entities::Value::Atom(name)) = definition
                    .fields
                    .iter()
                    .find(|(key, _)| key == "class")
                    .map(|(_, value)| value)
                {
                    let name = name.trim_matches('"').to_string();
                    if matches!(
                        name.as_str(),
                        "arkDoor" | "arkPickup" | "arkNPC" | "arkMover"
                    ) {
                        interactive += 1;
                    }
                    *class_totals.entry(name.clone()).or_default() += 1;
                    classes.insert(name);
                }
            } else if definition.kind == "component" {
                components.insert(definition.class.clone());
            }
        }
        levels.push((
            path,
            entities,
            classes.len(),
            components.len(),
            interactive,
            entities::player_start(&document).is_some(),
            bytes.len(),
        ));
    }
    levels.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.1.cmp(&a.1)));
    println!(
        "{} levels; {} distinct entity classes across all levels",
        levels.len(),
        class_totals.len()
    );
    println!("entities | classes | component types | selected interactives | start | MiB | level");
    for (path, entity_count, classes, components, interactive, start, bytes) in &levels {
        println!(
            "{:>8} | {:>7} | {:>15} | {:>21} | {:>5} | {:>4.1} | {path}",
            entity_count,
            classes,
            components,
            interactive,
            start,
            *bytes as f64 / (1024.0 * 1024.0)
        );
    }
    println!("Most common classes:");
    let mut totals: Vec<_> = class_totals.into_iter().collect();
    totals.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, count) in totals.into_iter().take(30) {
        println!("{count:>6} {name}");
    }
    Ok(())
}
