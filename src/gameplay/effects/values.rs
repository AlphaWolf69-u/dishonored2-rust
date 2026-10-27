//! Value producers used by authored effect-control graphs. Their callers retain
//! instance storage, event order and RNG ownership.
use super::variables::GraphValue;
use crate::{
    assets::entities::event_graph::{atom, Node},
    gameplay::event_graph::GraphInputs,
};

/// 904480/79bdc0/7ab820. Integer operations retain all32 bits; converting the
/// operands to floats would change comparisons and indices above2^24.
pub fn evaluate(
    inputs: &impl GraphInputs,
    node: &Node,
    port: &str,
    depth: usize,
) -> Result<Option<GraphValue>, String> {
    if depth > 64 {
        return Err("cyclic variable graph".into());
    }
    let value = match node.class.as_str() {
        "arkKisculeNodeAddInt" | "arkKisculeNodeCompareInt" => {
            if matches!(port, "m_a" | "m_b") {
                GraphValue::Int(integer_input(inputs, node, port, depth + 1)?)
            } else {
                let a = integer_input(inputs, node, "m_a", depth + 1)?;
                let b = integer_input(inputs, node, "m_b", depth + 1)?;
                match (node.class.as_str(), port) {
                    ("arkKisculeNodeAddInt", "Sum") => GraphValue::Int(a.wrapping_add(b)),
                    ("arkKisculeNodeCompareInt", "A == B") => GraphValue::Bool(a == b),
                    ("arkKisculeNodeCompareInt", "A &gt; B" | "A > B") => GraphValue::Bool(a > b),
                    ("arkKisculeNodeCompareInt", "A &lt; B" | "A < B") => GraphValue::Bool(a < b),
                    _ => return Ok(None),
                }
            }
        }
        "arkKisculeNodeListCount" if matches!(port, "Count" | "LastIndex") => {
            let count = if let Some((id, port)) = inputs.input(node, "List") {
                inputs.entity_references_inner(id, port, depth + 1)?.len() as i32
            } else {
                0
            };
            GraphValue::Int(if port == "Count" {
                count
            } else {
                count.wrapping_sub(1)
            })
        }
        "arkKisculeNodeGetItem" if matches!(port, "m_index" | "List" | "m_item") => {
            if port == "m_index" {
                GraphValue::Int(integer_input(inputs, node, "m_index", depth + 1)?)
            } else {
                let (id, input) = inputs
                    .input(node, "List")
                    .ok_or("List GetItem is not connected to a list")?;
                let values = inputs.entity_references_inner(id, input, depth + 1)?;
                if port == "List" {
                    GraphValue::Entities(values)
                } else if values.is_empty() {
                    // 7ab6b0 resets the typed element before checking count.
                    GraphValue::Entity(None)
                } else {
                    let index = integer_input(inputs, node, "m_index", depth + 1)?;
                    // Host memory safety: reject an invalid index. The typed
                    // handler's out-of-range failure path remains unconnected.
                    let value = usize::try_from(index)
                        .ok()
                        .and_then(|index| values.get(index))
                        .ok_or_else(|| format!("entity list index {index} is out of bounds"))?;
                    GraphValue::Entity(Some(value.clone()))
                }
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(value))
}

fn integer_input(
    inputs: &impl GraphInputs,
    node: &Node,
    name: &str,
    depth: usize,
) -> Result<i32, String> {
    if let Some((id, port)) = inputs.input(node, name) {
        inputs.integer(id, port, depth + 1)
    } else {
        // 79ba00 initializes both members of DoubleInt parameters tozero.
        atom(&node.params, name)
            .unwrap_or("0")
            .parse()
            .map_err(|_| format!("invalid integer {name}"))
    }
}
