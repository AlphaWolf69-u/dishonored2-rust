//! State/switch decision tree (1410ee5c0, 1410ee9a0, 1410eeaa0).
//! Tree records borrow the prepared definition; evaluation owns no bank data.
#[derive(Clone, Copy)]
struct Node { key: u32, value: u32, weight: u16, probability: u16 }
fn node(tree: &[u8], index: usize) -> Option<Node> {
    let bytes = tree.get(index.checked_mul(12)?..index.checked_add(1)?.checked_mul(12)?)?;
    Some(Node { key: u32::from_le_bytes(bytes[..4].try_into().unwrap()),
        value: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        weight: u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
        probability: u16::from_le_bytes(bytes[10..12].try_into().unwrap()) })
}
fn children(tree: &[u8], parent: Node) -> Option<&[u8]> {
    let first = (parent.value & 65535) as usize;
    let count = (parent.value >> 16) as usize;
    tree.get(first.checked_mul(12)?..first.checked_add(count)?.checked_mul(12)?)
}
fn best(tree: &[u8], current: Node, values: &[u32]) -> Option<Node> {
    let branch = children(tree, current)?;
    let (&value, rest) = values.split_first()?;
    let choose = |child| if rest.is_empty() { Some(child) } else { best(tree, child, rest) };
    let count = branch.len() / 12;
    let mut low = 0;
    let mut high = count;
    while low < high {
        let middle = low + (high - low) / 2;
        let child = node(branch, middle)?;
        match child.key.cmp(&value) {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => {
                if let Some(found) = choose(child) { return Some(found); }
                break;
            },
        }
    }
    let fallback = node(branch, 0)?;
    (fallback.key == 0 && value != 0).then(|| choose(fallback)).flatten()
}
fn candidates(tree: &[u8], current: Node, values: &[u32], output: &mut Vec<Node>) {
    let Some(branch) = children(tree, current) else { return; };
    let Some((&value, rest)) = values.split_first() else { return; };
    let mut visit = |child: Node| {
        if rest.is_empty() { output.push(child); }
        else { candidates(tree, child, rest, output); }
    };
    for index in 0..branch.len() / 12 {
        let child = node(branch, index).unwrap();
        if child.key == value { visit(child); break; }
    }
    if let Some(fallback) = node(branch, 0).filter(|child| child.key == 0 && value != 0) { visit(fallback); }
}
/// Mode zero prefers an exact branch with wildcard fallback; mode one
/// combines eligible branches using the original 100/weighted/zero cases.
/// The caller evaluates only when a subscribed group changes.
pub fn select(tree: &[u8], mode: u8, values: &[u32], mut random15: impl FnMut() -> u16) -> Option<u32> {
    let root = node(tree, 0)?;
    let selected = if values.is_empty() { root }
        else if mode == 0 { best(tree, root, values)? }
        else {
            let mut eligible = Vec::new();
            candidates(tree, root, values, &mut eligible);
            let full = eligible.iter().filter(|node| node.weight == 100).count();
            let zero = eligible.iter().filter(|node| node.weight == 0).count();
            if full != 0 {
                *eligible.iter().filter(|node| node.weight == 100).nth(usize::from(random15()) % full)?
            } else if zero < eligible.len() {
                let total = eligible.iter().map(|node| u32::from(node.weight)).sum::<u32>();
                let mut pick = u32::from(random15()) % total;
                *eligible.iter().find(|node| {
                    if pick < u32::from(node.weight) { true } else { pick -= u32::from(node.weight); false }
                })?
            } else {
                *eligible.iter().filter(|node| node.weight == 0).nth(usize::from(random15()) % zero.max(1))?
            }
        };
    if selected.probability < 100 {
        // 1410ee5c0 compares leaf probability * the tree's default 100.
        let draw = (f64::from(random15()) * (1.0 / 32767.0) * 10000.0) as u16;
        if selected.probability.wrapping_mul(100) <= draw { return None; }
    }
    (selected.value != 0).then_some(selected.value)
}
