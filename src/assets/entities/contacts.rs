//! Contact-system FX database (FUN_1400040a0 / FUN_1400041a0).
//! Entries retain the authored hash; material-pair lookup uses the game's
//! filename construction (140003cd0) and inheritance order (140002c60).
use super::{declaration_name_hash, parse_declaration, Parser, Token, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct ContactDatabase {
    effects: HashMap<u32, String>,
}

impl ContactDatabase {
    /// This table uses `"effect path" signed_hash` records rather than
    /// ordinary declaration fields. Reuse the declaration lexer so comments,
    /// quoted strings and whitespace follow the surrounding asset format.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut parser = Parser::new(text)?;
        parser.expect(Token::Open)?;
        if parser.atom()? != "m_fxTable" {
            return Err("contact database has no m_fxTable".into());
        }
        parser.expect(Token::Open)?;
        if parser.atom()? != "num" {
            return Err("contact FX table has no count".into());
        }
        parser.expect(Token::Equals)?;
        let count = parser.atom()?.parse::<usize>().map_err(|_| "invalid contact FX count")?;
        if parser.ahead == Some(Token::Semicolon) { parser.advance()?; }
        let mut effects = HashMap::new();
        let mut records = 0;
        while parser.ahead != Some(Token::Close) {
            let path = parser.atom()?.trim_matches('"').to_owned();
            let hash = parser.atom()?.parse::<i32>().map_err(|_| "invalid contact FX hash")? as u32;
            // The native hash table replaces an existing hash's descriptor.
            effects.insert(hash, path);
            records += 1;
            if parser.ahead == Some(Token::Semicolon) { parser.advance()?; }
        }
        parser.expect(Token::Close)?;
        parser.expect(Token::Close)?;
        if parser.ahead.is_some() { return Err("trailing contact database data".into()); }
        if records != count {
            return Err(format!("contact FX table declares {count} records, contains {records}"));
        }
        Ok(Self { effects })
    }

    /// Both chains start at the concrete material and end at its root.
    /// Source specificity takes precedence over target specificity: the
    /// target chain is exhausted before moving to the next source ancestor.
    pub fn effect(&self, source: &[String], target: &[String]) -> Option<&str> {
        for source in source {
            for target in target {
                let source = source.rsplit(['/', '\\']).next().unwrap_or(source);
                let target = target.rsplit(['/', '\\']).next().unwrap_or(target);
                let name = format!("contactsystem/{source}_{target}.fx").to_ascii_lowercase();
                if let Some(effect) = self.effects.get(&declaration_name_hash(name.as_bytes())) {
                    return Some(effect);
                }
            }
        }
        None
    }

    pub fn len(&self) -> usize { self.effects.len() }
    pub fn is_empty(&self) -> bool { self.effects.is_empty() }
}

/// Preserve declaration ancestry separately from the resolved material values:
/// contact lookup walks the parent pointers even when a child overrides nothing.
pub fn material_chain(
    name: &str,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut name = name.to_owned();
    while !name.is_empty() {
        if !seen.insert(name.clone()) { return Err(format!("physics material inheritance cycle at {name}")); }
        let path = format!("generated/decls/physicsmaterial/{name}.decl");
        let text = read(&path)?.ok_or_else(|| format!("missing physics material {path}"))?;
        let fields = parse_declaration(&text)?;
        result.push(name);
        name = match fields.iter().find(|(key, _)| key == "inherit") {
            Some((_, Value::Atom(parent))) => parent.trim_matches('"').to_owned(),
            Some(_) => return Err(format!("invalid physics material parent in {path}")),
            None => String::new(),
        };
    }
    Ok(result)
}
