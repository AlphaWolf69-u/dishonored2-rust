//! Batch-scoped source ownership for the processed entity declaration tree.
//!
//! Keep token ranges until inheritance has been processed: the ordinary
//! `Value` projection deliberately discards header and scalar token spacing.
use std::{borrow::Cow, collections::HashMap, ops::Range, sync::Arc};

#[derive(Clone, Debug)]
pub(super) struct Source {
    text: Arc<str>,
    range: Range<usize>,
}

impl Source {
    pub(super) fn new(text: Arc<str>) -> Self {
        let range = 0..text.len();
        Self { text, range }
    }
    pub(super) fn range(text: Arc<str>, range: Range<usize>) -> Self {
        Self { text, range }
    }
    fn text(&self) -> &str {
        &self.text[self.range.clone()]
    }
    fn slice(&self, span: &Range<usize>) -> &str {
        &self.text()[span.clone()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Word,
    Quoted,
    Open,
    Close,
    Equals,
    Semicolon,
}

#[derive(Clone, Debug)]
struct Token {
    kind: Kind,
    span: Range<usize>,
}

/// A node owns a source lease rather than copying the source for each token.
/// The lease is held only by the preparation tree, not by runtime components.
#[derive(Clone, Debug)]
pub(super) struct Node {
    source: Source,
    header: Vec<Token>,
    value_source: Source,
    modified: bool,
    value: NodeValue,
}

#[derive(Clone, Debug)]
enum NodeValue {
    Scalar(Vec<Token>),
    Scope(Vec<Node>),
    /// 140014850/140016550 retain primitives and kisculeDiff as separate
    /// text payloads; their contents do not enter reflected edit processing.
    RawScope(Range<usize>),
}

/// 140135f70 records the skipped range around whitespace and comments as a
/// token gap. 140149890 reduces a nonempty gap to one separating space.
fn token_gap(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if (bytes[at] != 0 && bytes[at] <= 0x20) || bytes[at] >= 0x80 {
            at += 1;
        } else if bytes[at..].starts_with(b"//") {
            at += 2;
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
        } else if bytes[at..].starts_with(b"/*") {
            at += 2;
            while at + 1 < bytes.len() && !bytes[at..].starts_with(b"*/") {
                at += 1;
            }
            if at + 1 >= bytes.len() {
                return false;
            }
            at += 2;
        } else {
            return false;
        }
    }
    true
}

/// Six little-endian base-64 digits used by 1400fe1f0 for edit-list IDs.
pub(super) fn encode_edit_id(value: u32) -> String {
    const DIGITS: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-_";
    (0..6)
        .map(|index| DIGITS[((value >> (index * 6)) & 63) as usize] as char)
        .collect()
}

fn decode_edit_id(value: &str) -> Result<u32, String> {
    if value.len() != 6 {
        return Err("edit-list ID must contain six digits".into());
    }
    let mut result = 0u32;
    for (index, byte) in value.bytes().enumerate() {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'A'..=b'Z' => byte - b'A' + 10,
            b'a'..=b'z' => byte - b'a' + 36,
            b'-' => 62,
            b'_' => 63,
            _ => return Err("invalid edit-list ID digit".into()),
        };
        result |= (digit as u32).wrapping_shl((index * 6) as u32);
    }
    Ok(result)
}

/// Decode the declaration lexer's ordinary quoted escapes.
fn decode_token(value: &str, quote: u8) -> Result<Vec<u8>, String> {
    let bytes = value.as_bytes();
    if bytes.len() < 2 || bytes[0] != quote || bytes[bytes.len() - 1] != quote {
        return Err("unsupported quoted scalar".into());
    }
    let mut decoded = Vec::new();
    let mut at = 1;
    while at < bytes.len() - 1 {
        let mut byte = bytes[at];
        at += 1;
        if byte == b'\\' {
            if at >= bytes.len() - 1 {
                return Err("incomplete quoted escape".into());
            }
            byte = bytes[at];
            at += 1;
            byte = match byte {
                b'a' => 7,
                b'b' => 8,
                b'f' => 12,
                b'n' => 10,
                b'r' => 13,
                b't' => 9,
                b'v' => 11,
                b'"' | b'\'' | b'?' | b'\\' => byte,
                b'0'..=b'9' => {
                    let mut number = (byte - b'0') as u32;
                    while at < bytes.len() - 1 && bytes[at].is_ascii_digit() {
                        number = number
                            .checked_mul(10)
                            .and_then(|value| value.checked_add((bytes[at] - b'0') as u32))
                            .filter(|value| *value <= i32::MAX as u32)
                            .ok_or_else(|| "overflowing quoted escape".to_owned())?;
                        at += 1;
                    }
                    number.min(255) as u8
                }
                b'x' => {
                    let mut number = 0u32;
                    while at < bytes.len() - 1 {
                        let digit = match bytes[at] {
                            b'0'..=b'9' => (bytes[at] - b'0') as u32,
                            b'A'..=b'Z' => (bytes[at] - b'A' + 10) as u32,
                            b'a'..=b'z' => (bytes[at] - b'a' + 10) as u32,
                            _ => break,
                        };
                        number = number
                            .checked_mul(16)
                            .and_then(|value| value.checked_add(digit))
                            .filter(|value| *value <= i32::MAX as u32)
                            .ok_or_else(|| "overflowing quoted escape".to_owned())?;
                        at += 1;
                    }
                    number.min(255) as u8
                }
                _ => return Err("unsupported quoted escape".into()),
            };
        }
        if byte == 0 {
            return Err("zero escape requires native string semantics".into());
        }
        decoded.push(byte);
    }
    Ok(decoded)
}

pub(super) fn decode_quoted(value: &str) -> Result<Vec<u8>, String> {
    decode_token(value, b'"')
}

/// Apply the writer's 140140e20 quoting to a decoded token.
fn quote_decoded(decoded: &[u8]) -> Vec<u8> {
    let mut result = vec![b'"'];
    for &byte in decoded {
        let escaped = match byte {
            7 => Some(b'a'),
            8 => Some(b'b'),
            9 => Some(b't'),
            10 => Some(b'n'),
            11 => Some(b'v'),
            12 => Some(b'f'),
            13 => Some(b'r'),
            b'"' | b'\'' | b'?' | b'\\' => Some(byte),
            _ => None,
        };
        if let Some(escaped) = escaped {
            result.push(b'\\');
            result.push(escaped);
        } else {
            result.push(byte);
        }
    }
    result.push(b'"');
    result
}

fn normalize_quoted(value: &str) -> Result<Vec<u8>, String> {
    Ok(quote_decoded(&decode_quoted(value)?))
}

impl Node {
    /// Native declaration headers contain up to two type/name prefixes and a
    /// field name. Adjacent indexed-name tokens stay in the same group.
    fn header_groups(&self) -> Result<Vec<&str>, String> {
        let mut groups = Vec::new();
        let mut start = self.header[0].span.start;
        let mut end = self.header[0].span.end;
        for token in &self.header[1..] {
            let gap = &self.source.text()[end..token.span.start];
            if !token_gap(gap) {
                return Err("comment in declaration header requires native lexer handling".into());
            }
            if !gap.is_empty() {
                groups.push(&self.source.text()[start..end]);
                start = token.span.start;
            }
            end = token.span.end;
        }
        groups.push(&self.source.text()[start..end]);
        if groups.len() > 3 {
            return Err("unsupported declaration header".into());
        }
        Ok(groups)
    }

    pub(super) fn field_name(&self) -> Result<Cow<'_, str>, String> {
        let groups = self.header_groups()?;
        let name = groups
            .last()
            .copied()
            .ok_or_else(|| "missing field name".to_owned())?;
        if name.starts_with('"') {
            return String::from_utf8(decode_quoted(name)?)
                .map(Cow::Owned)
                .map_err(|_| "non-UTF-8 field name".into());
        }
        if name.starts_with('\'') {
            return String::from_utf8(decode_token(name, b'\'')?)
                .map(Cow::Owned)
                .map_err(|_| "non-UTF-8 field name".into());
        }
        if let Some((base, suffix)) = name.split_once('[') {
            let index = suffix
                .strip_suffix(']')
                .ok_or_else(|| "invalid indexed name".to_owned())?;
            if base.is_empty()
                || !base
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err("unsupported indexed field name".into());
            }
            if index.starts_with('"') {
                let decoded = String::from_utf8(decode_quoted(index)?)
                    .map_err(|_| "non-UTF-8 indexed name".to_owned())?;
                let normalized = format!("{base}[\"{decoded}\"]");
                if normalized != name {
                    return Ok(Cow::Owned(normalized));
                }
            } else if base == "enumItem"
                && index.bytes().next().is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
            {
                // Enum-array keys are symbolic until the reflected field's
                // enum table resolves them. Preserve the header, as for the
                // installed PLAYER_CORVO/PLAYER_EMILY character-name array.
                if !index.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
                    return Err(format!("unsupported enum field index {index:?} in {name:?}"));
                }
            } else {
                let value = index
                    .parse::<i32>()
                    .map_err(|_| format!("unsupported numeric field index {index:?} in {name:?}"))?;
                if value.to_string() != index {
                    return Ok(Cow::Owned(format!("{base}[{value}]")));
                }
            }
        } else if !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err("field name requires native token handling".into());
        }
        Ok(Cow::Borrowed(name))
    }

    pub(super) fn scalar(&self) -> Option<&str> {
        let NodeValue::Scalar(tokens) = &self.value else {
            return None;
        };
        let [token] = tokens.as_slice() else {
            return None;
        };
        Some(self.value_source.slice(&token.span))
    }
}

/// 14000c460 visits later siblings and children first, then swaps an
/// `item`-prefixed node followed by `num`. Work backwards for longer runs.
fn reorder_array_counts(nodes: &mut Vec<Node>) -> Result<(), String> {
    for node in nodes.iter_mut().rev() {
        if let NodeValue::Scope(children) = &mut node.value {
            reorder_array_counts(children)?;
        }
    }
    for index in (0..nodes.len().saturating_sub(1)).rev() {
        if nodes[index].field_name()?.starts_with("item") && nodes[index + 1].field_name()? == "num"
        {
            nodes.swap(index, index + 1);
        }
    }
    Ok(())
}

/// 140153be0 prunes array entries after every inherited declaration layer.
/// A non-simple count or index cannot be interpreted by this partial port.
fn prune_arrays(nodes: &mut Vec<Node>) -> Result<(), String> {
    let mut count = None;
    for node in nodes.iter() {
        if node.field_name()?.starts_with("num") {
            count = Some(
                node.scalar()
                    .ok_or_else(|| "unsupported array count".to_owned())?
                    .parse::<i32>()
                    .map_err(|_| "invalid array count".to_owned())?,
            );
            break;
        }
    }
    if let Some(count) = count {
        let mut error = None;
        nodes.retain(|node| {
            let name = match node.field_name() {
                Ok(name) => name,
                Err(problem) => {
                    error = Some(problem);
                    return true;
                }
            };
            let Some(index) = name
                .strip_prefix("item[")
                .and_then(|name| name.strip_suffix(']'))
            else {
                return true;
            };
            if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
                error = Some(format!("unsupported array index {name}"));
                return true;
            }
            match index.parse::<i32>() {
                Ok(index) => index < count,
                Err(_) => {
                    error = Some(format!("unsupported array index {name}"));
                    true
                }
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
    }
    for node in nodes {
        if let NodeValue::Scope(children) = &mut node.value {
            prune_arrays(children)?;
        }
    }
    Ok(())
}

fn merge_tree_nodes(target: &mut Vec<Node>, incoming: &[Node]) -> Result<(), String> {
    for node in incoming {
        let name = node.field_name()?;
        let mut matched = None;
        for (index, old) in target.iter().enumerate() {
            if old.field_name()? == name {
                matched = Some(index);
                break;
            }
        }
        if let Some(index) = matched {
            let old = &mut target[index];
            match (&mut old.value, &node.value) {
                (NodeValue::Scope(children), NodeValue::Scope(layer)) => {
                    merge_tree_nodes(children, layer)?;
                    old.source = node.source.clone();
                    old.header = node.header.clone();
                    old.modified = node.modified;
                }
                _ => *old = node.clone(),
            }
        } else {
            target.push(node.clone());
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum EditEntry {
    Add(u32),
    Override(u32),
    Item(u32),
}

fn edit_entry(name: &str) -> Result<Option<EditEntry>, String> {
    for (prefix, make) in [
        ("item_add[\"", EditEntry::Add as fn(u32) -> EditEntry),
        (
            "item_override[\"",
            EditEntry::Override as fn(u32) -> EditEntry,
        ),
    ] {
        if let Some(value) = name
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix("\"]"))
        {
            return Ok(Some(make(decode_edit_id(value)?)));
        }
    }
    if let Some(value) = name
        .strip_prefix("item[")
        .and_then(|value| value.strip_suffix(']'))
    {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(format!("unsupported edit-list item {name}"));
        }
        let index = value
            .parse::<i32>()
            .map_err(|_| format!("invalid edit-list item {name}"))?;
        return Ok(Some(EditEntry::Item(index.wrapping_add(1) as u32)));
    }
    if name.starts_with("item_add[")
        || name.starts_with("item_override[")
        || name.starts_with("item[")
    {
        return Err(format!("unsupported edit-list entry {name}"));
    }
    Ok(None)
}

/// Apply the reverse sibling walk in 1400fe1f0 after one inheritance layer.
/// The complete parser must still supply native-normalized field names and
/// values before this operation may determine runtime declaration identity.
fn apply_edit_lists(nodes: &mut Vec<Node>, inside_inline: bool) -> Result<(), String> {
    let mut index = nodes.len();
    while index > 0 {
        index -= 1;
        let inline = inside_inline || nodes[index].field_name()? == "inlineDecl";
        if let NodeValue::Scope(children) = &mut nodes[index].value {
            apply_edit_lists(children, inline)?;
        }
        let Some(entry) = edit_entry(&nodes[index].field_name()?)? else {
            continue;
        };
        let (id, is_override) = match entry {
            EditEntry::Add(_) => continue,
            EditEntry::Override(id) => (id, true),
            EditEntry::Item(id) => (id, false),
        };
        let mut matched = None;
        for (candidate, node) in nodes.iter().enumerate() {
            if candidate == index {
                continue;
            }
            let same = match edit_entry(&node.field_name()?)? {
                Some(EditEntry::Add(value) | EditEntry::Item(value)) => value == id,
                _ => false,
            };
            if same {
                matched = Some(candidate);
                break;
            }
        }
        let Some(candidate) = matched else {
            if is_override && !inside_inline {
                nodes.remove(index);
            }
            continue;
        };
        let source = nodes[index].clone();
        let target = &mut nodes[candidate];
        match (&mut target.value, &source.value) {
            (NodeValue::Scope(old), NodeValue::Scope(new)) => {
                if new.is_empty() {
                    old.clear();
                }
                let inherited = |nodes: &[Node]| -> Result<Option<String>, String> {
                    let Some(first) = nodes.first() else {
                        return Ok(None);
                    };
                    if first.field_name()? != "inheritedDecl" {
                        return Ok(None);
                    }
                    let value = first
                        .scalar()
                        .ok_or_else(|| "unsupported inherited declaration value".to_owned())?;
                    if value.starts_with('"') {
                        let normalized = normalize_quoted(value)?;
                        Ok(Some(String::from_utf8(normalized).map_err(|_| {
                            "invalid inherited declaration text".to_owned()
                        })?))
                    } else {
                        Ok(Some(value.to_owned()))
                    }
                };
                if matches!((inherited(old)?, inherited(new)?), (Some(a), Some(b)) if a != b) {
                    old.clear();
                }
                merge_tree_nodes(old, new)?;
            }
            _ => {
                target.value = source.value.clone();
                target.value_source = source.value_source.clone();
            }
        }
        nodes.remove(index);
    }
    Ok(())
}

pub(super) fn merge_layer(target: &mut Vec<Node>, incoming: &[Node]) -> Result<(), String> {
    merge_tree_nodes(target, incoming)?;
    prune_arrays(target)?;
    apply_edit_lists(target, false)
}

fn write_scalar(node: &Node, tokens: &[Token], output: &mut Vec<u8>) -> Result<(), String> {
    let start = output.len();
    let mut previous = None;
    for token in tokens {
        if !matches!(token.kind, Kind::Word | Kind::Quoted) {
            return Err("unsupported scalar punctuation".into());
        }
        if let Some(end) = previous {
            let gap = &node.value_source.text()[end..token.span.start];
            if !token_gap(gap) {
                return Err("unsupported scalar token gap".into());
            }
            if !gap.is_empty() && output.len() > start {
                output.push(b' ');
            }
        }
        let text = node.value_source.slice(&token.span);
        if token.kind == Kind::Quoted {
            if text.starts_with('"') {
                // 140149890 replaces its accumulated scalar when a string
                // token is encountered, then emits the escaped form.
                output.truncate(start);
                output.extend(normalize_quoted(text)?);
            } else {
                // Single-quoted tokens have lexer type 2 and append decoded
                // bytes without the type-1 quoting transformation.
                output.extend(decode_token(text, b'\'')?);
            }
        } else {
            output.extend_from_slice(text.as_bytes());
        }
        previous = Some(token.span.end);
    }
    Ok(())
}

/// Restricted 140154100 writer for ordinary headers and scalar tokens. The
/// complete writer must additionally handle native indexed headers,
/// escapes and edit-list transformations before this can be used as a key.
fn write_simple_nodes(nodes: &[Node], depth: usize, output: &mut Vec<u8>) -> Result<(), String> {
    for node in nodes {
        let groups = node.header_groups()?;
        let name = node.field_name()?;
        output.push(b'\n');
        output.extend(std::iter::repeat_n(b'\t', depth));
        for prefix in &groups[..groups.len() - 1] {
            let prefix = if prefix.starts_with('"') || prefix.starts_with('\'') {
                Cow::Owned(decode_token(prefix, prefix.as_bytes()[0])?)
            } else {
                Cow::Borrowed(prefix.as_bytes())
            };
            if prefix.is_empty()
                || !prefix
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                return Err("unsupported typed writer prefix".into());
            }
            output.push(b'"');
            output.extend_from_slice(&prefix);
            output.extend_from_slice(b"\" ");
        }
        if name
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        {
            output.extend_from_slice(name.as_bytes());
        } else {
            output.extend(quote_decoded(name.as_bytes()));
        }
        output.extend_from_slice(b" = ");
        if node.modified {
            output.extend_from_slice(b"! ");
        }
        match &node.value {
            NodeValue::RawScope(span) => {
                output.extend_from_slice(node.value_source.slice(span).as_bytes());
            }
            NodeValue::Scope(children) => {
                output.push(b'{');
                write_simple_nodes(children, depth + 1, output)?;
                output.push(b'\n');
                output.extend(std::iter::repeat_n(b'\t', depth));
                output.push(b'}');
            }
            NodeValue::Scalar(tokens) => {
                write_scalar(node, tokens, output)?;
                output.push(b';');
            }
        }
    }
    Ok(())
}

/// Form the inline body seen by 1407040f0 after the processed slot is written.
/// This is not yet an admission API: the parser and writer still reject native
/// forms whose normalization has not been ported.
fn find_node<'a>(nodes: &'a [Node], name: &str) -> Result<Option<&'a Node>, String> {
    for node in nodes {
        if node.field_name()? == name {
            return Ok(Some(node));
        }
    }
    Ok(None)
}

/// Project a single reflected scope without retaining or projecting unrelated
/// source fields. The same processed tree and edit-list operations own it.
pub(super) fn project_named_scope(nodes: &[Node], name: &str)
    -> Result<Option<Vec<(String, super::Value)>>, String> {
    let Some(node) = find_node(nodes, name)? else { return Ok(None) };
    match &node.value {
        NodeValue::Scope(children) => project_scope(children).map(Some),
        _ => Err(format!("{name} is not a scope")),
    }
}

/// Project selected fields in authored order without decoding unrelated
/// fields in the same declaration layer.
pub(super) fn project_named_fields(nodes: &[Node], names: &[&str])
    -> Result<Vec<(String, super::Value)>, String> {
    let mut result = Vec::new();
    for node in nodes {
        let name = node.field_name()?;
        if names.iter().any(|candidate| *candidate == name.as_ref()) {
            result.extend(project_scope(std::slice::from_ref(node))?);
        }
    }
    Ok(result)
}

pub(super) fn project_scope(nodes: &[Node]) -> Result<Vec<(String, super::Value)>, String> {
    let mut fields = Vec::with_capacity(nodes.len());
    for node in nodes {
        let name = node.field_name()?.into_owned();
        let value = match &node.value {
            NodeValue::RawScope(span) => super::Value::Atom(node.value_source.slice(span).to_owned()),
            NodeValue::Scope(children) => super::Value::Scope(project_scope(children)?),
            NodeValue::Scalar(tokens) => {
                for token in tokens.iter().filter(|token| token.kind == Kind::Quoted) {
                    let text = node.value_source.slice(&token.span);
                    if tokens.len() != 1
                        || !text.starts_with('"')
                        || normalize_quoted(text)? != text.as_bytes()
                    {
                        return Err("processed scalar requires a decoded value owner".into());
                    }
                }
                let mut value = Vec::new();
                write_scalar(node, tokens, &mut value)?;
                super::Value::Atom(
                    String::from_utf8(value)
                        .map_err(|_| "non-UTF-8 processed scalar".to_owned())?,
                )
            }
        };
        fields.push((name, value));
    }
    Ok(fields)
}

/// 1400fdc20 enumerates slot nodes in their processed sibling order. The
/// canonical slot helper maps numeric indices to the same native edit ID.
pub(super) fn component_entries(
    nodes: &[Node],
    expanded_inheritance: bool,
) -> Result<Vec<(String, Vec<(String, super::Value)>)>, String> {
    let Some(components) = find_node(nodes, "m_componentDecls")? else {
        return Ok(Vec::new());
    };
    let NodeValue::Scope(entries) = &components.value else {
        return Err("component list is not a scope".into());
    };
    let mut result = Vec::new();
    for node in entries {
        let name = node.field_name()?;
        if !expanded_inheritance {
            if name == "item_begin" {
                if !result.is_empty() || !matches!(node.scalar(), Some("true" | "1")) {
                    return Err("direct component list requires native item_begin handling".into());
                }
                continue;
            }
            if !matches!(edit_entry(&name)?, Some(EditEntry::Add(_))) {
                return Err(format!("direct component list requires native operation handling: {name}"));
            }
        }
        let Some(slot) = super::spawn::component_slot_identity(&name)? else {
            continue;
        };
        if !expanded_inheritance && result.iter().any(|(existing, _)| existing == &slot) {
            return Err(format!("duplicate direct component slot {slot}"));
        }
        let NodeValue::Scope(fields) = &node.value else {
            return Err(format!("component slot {name} is not a scope"));
        };
        let mut fields = project_scope(fields)?;
        // 00c710: a mapComponentDecl entry discards every sibling field
        // except mapComponentDecl. 1401e0 is inequality, and +b8 is the
        // parent link (153b10). Inherited inline edits do not survive this
        // branch and must not produce a second inline declaration identity.
        if super::field(&fields, "mapComponentDecl").is_some() {
            fields.retain(|(name, _)| name == "mapComponentDecl");
        }
        result.push((slot, fields));
    }
    Ok(result)
}

pub(super) fn inline_key(nodes: &[Node], slot: &str) -> Result<Option<Vec<u8>>, String> {
    let components = find_node(nodes, "m_componentDecls")?
        .ok_or_else(|| "processed component list is missing".to_owned())?;
    let NodeValue::Scope(entries) = &components.value else {
        return Err("component list is not a scope".into());
    };
    let mut entry = None;
    for node in entries {
        let name = node.field_name()?;
        let authored_slot = name
            .strip_prefix("item_add[")
            .or_else(|| name.strip_prefix("item_override["))
            .or_else(|| name.strip_prefix("item["))
            .and_then(|value| value.strip_suffix(']'))
            .map(|value| value.trim_matches('"'));
        let numeric_matches = if let Some(EditEntry::Item(id)) = edit_entry(&name)? {
            encode_edit_id(id) == slot
        } else {
            false
        };
        if name == slot || authored_slot == Some(slot) || numeric_matches {
            entry = Some(node);
            break;
        }
    }
    let entry = entry.ok_or_else(|| format!("processed component slot {slot} is missing"))?;
    let NodeValue::Scope(fields) = &entry.value else {
        return Err("component entry is not a scope".into());
    };
    // 00c710's map-declaration normalization precedes 703ca0 pointer parsing.
    if find_node(fields, "mapComponentDecl")?.is_some() { return Ok(None); }
    let Some(inline) = find_node(fields, "inlineDecl")? else {
        return Ok(None);
    };
    let NodeValue::Scope(fields) = &inline.value else {
        return Err("inline declaration is not a scope".into());
    };
    if fields.is_empty() {
        return Ok(None);
    }
    let mut fields = fields.clone();
    reorder_array_counts(&mut fields)?;
    if let Some(Node {
        value: NodeValue::Scope(children),
        ..
    }) = fields.first_mut()
    {
        prune_arrays(children)?;
    }
    let mut output = Vec::new();
    write_simple_nodes(&fields, 2, &mut output)?;
    output.extend_from_slice(b"\n\t");
    Ok(Some(output))
}

/// One preparation batch owns both the inherited tree cache and its source
/// leases. The loader must borrow the same catalog/source owner used by the
/// entity declaration resolver, so that admission does not reread archives.
#[derive(Default)]
pub(super) struct Resolver {
    parents: HashMap<String, Result<Arc<Vec<Node>>, String>>,
}

impl Resolver {
    pub(super) fn resolve<F>(&mut self, fields: &[Node], mut load: F) -> Result<Vec<Node>, String>
    where
        F: FnMut(&str) -> Result<Arc<str>, String>,
    {
        self.resolve_with_layer(fields, &mut load, |_| Ok(()))
    }

    /// Declaration resource owners can register referenced resources as each
    /// real inherited layer is admitted, before later overrides replace it.
    pub(super) fn resolve_with_layer<F, V>(&mut self, fields: &[Node], mut load: F,
        mut visit: V) -> Result<Vec<Node>, String>
    where
        F: FnMut(&str) -> Result<Arc<str>, String>,
        V: FnMut(&[Node]) -> Result<(), String>,
    {
        if let Some(flag) = find_node(fields, "expandInheritance")? {
            match flag.scalar() {
                Some("true" | "1") => {}
                Some("false" | "0") => {
                    // 140016640 copies the authored edit text directly when
                    // expansion is disabled. The parent remains available to
                    // the declaration owner for metadata, but its edit tree
                    // is not merged into this text.
                    let Some(edit) = find_node(fields, "edit")? else { return Ok(Vec::new()) };
                    let NodeValue::Scope(nodes) = &edit.value else {
                        return Err("entity edit is not a scope".into());
                    };
                    visit(nodes)?;
                    return Ok(nodes.clone());
                }
                _ => return Err("invalid expandInheritance value".into()),
            }
        }
        self.resolve_fields(fields, &mut Vec::new(), &mut load, &mut visit)
    }

    fn resolve_fields<F, V>(
        &mut self,
        fields: &[Node],
        path: &mut Vec<String>,
        load: &mut F,
        visit: &mut V,
    ) -> Result<Vec<Node>, String>
    where
        F: FnMut(&str) -> Result<Arc<str>, String>,
        V: FnMut(&[Node]) -> Result<(), String>,
    {
        let parent = find_node(fields, "inherit")?
            .map(|node| {
                let value = node
                    .scalar()
                    .ok_or_else(|| "unsupported inherit value".to_owned())?;
                let value = if value.starts_with('"') {
                    let bytes = normalize_quoted(value)?;
                    if bytes != value.as_bytes() {
                        return Err("inherit name requires native normalization".to_owned());
                    }
                    value
                        .strip_prefix('"')
                        .and_then(|value| value.strip_suffix('"'))
                        .ok_or_else(|| "invalid inherit name".to_owned())?
                } else {
                    value
                };
                Ok::<_, String>(value)
            })
            .transpose()?;
        let mut tree =
            match parent.filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL")) {
                Some(name) => self.parent(name, path, load, visit)?.as_ref().clone(),
                None => Vec::new(),
            };
        if let Some(edit) = find_node(fields, "edit")? {
            let NodeValue::Scope(fields) = &edit.value else {
                return Err("entity edit is not a scope".into());
            };
            visit(fields)?;
            merge_layer(&mut tree, fields)?;
        }
        Ok(tree)
    }

    fn parent<F, V>(
        &mut self,
        name: &str,
        path: &mut Vec<String>,
        load: &mut F,
        visit: &mut V,
    ) -> Result<Arc<Vec<Node>>, String>
    where
        F: FnMut(&str) -> Result<Arc<str>, String>,
        V: FnMut(&[Node]) -> Result<(), String>,
    {
        if let Some(cached) = self.parents.get(name) {
            return cached.clone();
        }
        if path.len() >= 32 || path.iter().any(|entry| entry == name) {
            return Err(format!(
                "entity declaration inheritance cycle or depth limit at {name}"
            ));
        }
        path.push(name.to_owned());
        let result = (|| {
            let source = load(name)?;
            let fields = parse(Source::new(source))?;
            self.resolve_fields(&fields, path, load, visit).map(Arc::new)
        })();
        path.pop();
        self.parents.insert(name.to_owned(), result.clone());
        result
    }
}

struct Parser {
    source: Source,
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn new(source: Source) -> Result<Self, String> {
        if source.text().contains('\u{fffd}') {
            return Err("lossy declaration source cannot establish byte identity".into());
        }
        let bytes = source.text().as_bytes();
        if bytes.contains(&0) {
            return Err("embedded zero requires native input termination".into());
        }
        let mut tokens = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] <= 0x20 || bytes[at] >= 0x80 {
                at += 1;
                continue;
            }
            if bytes[at..].starts_with(b"//") {
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
                continue;
            }
            if bytes[at..].starts_with(b"/*") {
                let start = at;
                at += 2;
                while at + 1 < bytes.len() && !bytes[at..].starts_with(b"*/") {
                    at += 1;
                }
                if at + 1 >= bytes.len() {
                    return Err(format!("unterminated comment at byte {start}"));
                }
                at += 2;
                continue;
            }
            let start = at;
            let kind = match bytes[at] {
                b'{' => {
                    at += 1;
                    Kind::Open
                }
                b'}' => {
                    at += 1;
                    Kind::Close
                }
                b'=' => {
                    at += 1;
                    Kind::Equals
                }
                b';' => {
                    at += 1;
                    Kind::Semicolon
                }
                b'!' => {
                    at += 1;
                    Kind::Word
                }
                b'"' | b'\'' => {
                    let quote = bytes[at];
                    at += 1;
                    while at < bytes.len() && bytes[at] != quote {
                        if bytes[at] == b'\n' {
                            return Err(format!("newline in quoted token at byte {start}"));
                        }
                        if bytes[at] == b'\\' {
                            at += 1;
                        }
                        at += 1;
                    }
                    if at >= bytes.len() {
                        return Err(format!("unterminated quote at byte {start}"));
                    }
                    at += 1;
                    Kind::Quoted
                }
                _ => {
                    while at < bytes.len()
                        && bytes[at] > 0x20
                        && bytes[at] < 0x80
                        && !matches!(bytes[at], b'{' | b'}' | b'=' | b';' | b'!' | b'"' | b'\'')
                        && !bytes[at..].starts_with(b"//")
                        && !bytes[at..].starts_with(b"/*")
                    {
                        at += 1;
                    }
                    if at == start {
                        return Err(format!("invalid token at byte {start}"));
                    }
                    Kind::Word
                }
            };
            tokens.push(Token {
                kind,
                span: start..at,
            });
        }
        Ok(Self {
            source,
            tokens,
            at: 0,
        })
    }

    fn scope(&mut self, depth: usize) -> Result<Vec<Node>, String> {
        if depth > 128 {
            return Err("declaration nesting exceeds 128".into());
        }
        let mut nodes = Vec::new();
        loop {
            match self.tokens.get(self.at).map(|token| token.kind) {
                Some(Kind::Close) => {
                    self.at += 1;
                    return Ok(nodes);
                }
                None => return Err("unterminated declaration scope".into()),
                _ => {}
            }
            let mut header = Vec::new();
            while matches!(
                self.tokens.get(self.at).map(|token| token.kind),
                Some(Kind::Word | Kind::Quoted)
            ) {
                header.push(self.tokens[self.at].clone());
                self.at += 1;
            }
            if header.is_empty() {
                return Err("missing declaration field name".into());
            }
            if self
                .tokens
                .get(self.at)
                .is_some_and(|token| token.kind == Kind::Equals)
            {
                self.at += 1;
            }
            let modified = self.tokens.get(self.at).is_some_and(|token| {
                token.kind == Kind::Word && self.source.slice(&token.span) == "!"
            });
            if modified {
                self.at += 1;
            }
            let value = if self
                .tokens
                .get(self.at)
                .is_some_and(|token| token.kind == Kind::Open)
            {
                let raw = depth == 0 && header.len() == 1
                    && matches!(self.source.slice(&header[0].span), "primitives" | "kisculeDiff");
                if raw {
                    let start = self.tokens[self.at].span.start;
                    let mut nesting = 0usize;
                    let end = loop {
                        let token = self.tokens.get(self.at)
                            .ok_or("unterminated raw entity-definition property")?;
                        self.at += 1;
                        match token.kind {
                            Kind::Open => nesting += 1,
                            Kind::Close => {
                                nesting -= 1;
                                if nesting == 0 { break token.span.end; }
                            }
                            _ => {}
                        }
                    };
                    NodeValue::RawScope(start..end)
                } else {
                    self.at += 1;
                    NodeValue::Scope(self.scope(depth + 1)?)
                }
            } else {
                let start = self.at;
                while !matches!(
                    self.tokens.get(self.at).map(|token| token.kind),
                    None | Some(Kind::Semicolon | Kind::Close)
                ) {
                    if !matches!(self.tokens[self.at].kind, Kind::Word | Kind::Quoted) {
                        return Err("unsupported scalar syntax".into());
                    }
                    self.at += 1;
                }
                if start == self.at {
                    return Err("missing declaration field value".into());
                }
                NodeValue::Scalar(self.tokens[start..self.at].to_vec())
            };
            if self
                .tokens
                .get(self.at)
                .is_some_and(|token| token.kind == Kind::Semicolon)
            {
                self.at += 1;
            }
            nodes.push(Node {
                source: self.source.clone(),
                header,
                value_source: self.source.clone(),
                modified,
                value,
            });
        }
    }
}

pub(super) fn parse(source: Source) -> Result<Vec<Node>, String> {
    let mut parser = Parser::new(source)?;
    if !parser
        .tokens
        .first()
        .is_some_and(|token| token.kind == Kind::Open)
    {
        return Err("missing declaration root scope".into());
    }
    parser.at = 1;
    let result = parser.scope(0)?;
    if parser.at != parser.tokens.len() {
        return Err("trailing declaration data".into());
    }
    Ok(result)
}
