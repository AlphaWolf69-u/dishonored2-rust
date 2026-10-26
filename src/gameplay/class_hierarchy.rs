//! Runtime class hierarchy materialization and type IDs (199530/1995f0).
//! Inputs are resolved base-class handles and the actual registration-list order;
//! string interning and class registration remain their own metadata producers.

pub struct ClassNode {
    pub base: Option<usize>,
    pub initialized: bool,
    pub type_id: i32,
    pub last_type_id: i32,
    parent: Option<usize>,
    children: Vec<usize>,
}

pub struct ClassHierarchy {
    nodes: Vec<ClassNode>,
    registration_order: Vec<usize>,
    roots: Vec<usize>,
    by_name: Vec<usize>,
    by_id: Vec<usize>,
    initialized: bool,
    type_bits: i32,
}
impl ClassHierarchy {
    pub(crate) fn insert_class(
        &mut self,
        base: Option<usize>,
        position: usize,
    ) -> Result<usize, String> {
        if position > self.registration_order.len() || base.is_some_and(|v| v >= self.nodes.len()) {
            return Err("class registration links are invalid".into());
        }
        let index = self.nodes.len();
        self.nodes.push(ClassNode {
            base,
            initialized: false,
            type_id: 0,
            last_type_id: 0,
            parent: None,
            children: Vec::new(),
        });
        self.registration_order.insert(position, index);
        Ok(index)
    }
    pub(crate) fn set_base(&mut self, index: usize, base: usize) {
        self.nodes[index].base = Some(base);
    }
    pub fn registration_order(&self) -> &[usize] {
        &self.registration_order
    }

    pub fn new(bases: Vec<Option<usize>>, registration_order: Vec<usize>) -> Result<Self, String> {
        let count = bases.len();
        if count > i32::MAX as usize || registration_order.len() != count {
            return Err("class registration count is invalid".into());
        }
        let mut seen = vec![false; count];
        for &index in &registration_order {
            let used = seen
                .get_mut(index)
                .ok_or("registered class handle is missing")?;
            if std::mem::replace(used, true) {
                return Err("class registration list contains a repeated handle".into());
            }
        }
        if bases.iter().flatten().any(|&index| index >= count) {
            return Err("base class handle is missing".into());
        }
        Ok(Self {
            nodes: bases
                .into_iter()
                .map(|base| ClassNode {
                    base,
                    initialized: false,
                    type_id: 0,
                    last_type_id: 0,
                    parent: None,
                    children: Vec::new(),
                })
                .collect(),
            registration_order,
            roots: Vec::new(),
            by_name: Vec::new(),
            by_id: Vec::new(),
            initialized: false,
            type_bits: 0,
        })
    }

    /// 199910 detaches an intrusive node, preserving its children. Vec removal
    /// supplies the same ordered sibling unlink under exclusive ownership.
    fn detach(&mut self, index: usize) -> Result<(), String> {
        let parent = self.nodes.get(index).ok_or("class node is missing")?.parent;
        let siblings = match parent {
            Some(parent) => &mut self.nodes[parent].children,
            None => &mut self.roots,
        };
        if let Some(at) = siblings.iter().position(|&value| value == index) {
            siblings.remove(at);
        } else if parent.is_some() {
            return Err("class node is absent from its parent's children".into());
        }
        self.nodes[index].parent = None;
        Ok(())
    }

    /// Complete199530: initialize the base first, prepend this node, then count
    /// it in every ancestor. Previously initialized nodes remain unchanged.
    pub fn initialize_node(&mut self, index: usize) -> Result<(), String> {
        let mut visiting = vec![false; self.nodes.len()];
        self.initialize_node_inner(index, &mut visiting)
    }
    fn initialize_node_inner(&mut self, index: usize, visiting: &mut [bool]) -> Result<(), String> {
        let node = self.nodes.get(index).ok_or("class node is missing")?;
        if node.initialized {
            return Ok(());
        }
        if std::mem::replace(&mut visiting[index], true) {
            return Err("cyclic base-class hierarchy".into());
        }
        let base = node.base;
        if let Some(base) = base {
            self.initialize_node_inner(base, visiting)?;
        }
        self.detach(index)?;
        match base {
            Some(base) => self.nodes[base].children.insert(0, index),
            None => self.roots.insert(0, index),
        }
        self.nodes[index].parent = base;
        let mut ancestor = base;
        while let Some(index) = ancestor {
            self.nodes[index].last_type_id = self.nodes[index].last_type_id.wrapping_add(1);
            ancestor = self.nodes[index].base;
        }
        self.nodes[index].initialized = true;
        visiting[index] = false;
        Ok(())
    }

    /// Complete1995f0's logical hierarchy/table state. Its logging calls return,
    /// including the already-initialized branch. IDs follow preorder of the
    /// materialized tree, not alphabetical class names or document order.
    pub fn initialize(&mut self) -> Result<bool, String> {
        if self.initialized {
            return Ok(false);
        }
        let mut visiting = vec![false; self.nodes.len()];
        for at in 0..self.registration_order.len() {
            self.initialize_node_inner(self.registration_order[at], &mut visiting)?;
        }
        let mut pending: Vec<_> = self.roots.iter().rev().copied().collect();
        self.by_id.clear();
        while let Some(index) = pending.pop() {
            let id = self.by_id.len() as i32;
            let node = &mut self.nodes[index];
            node.type_id = id;
            node.last_type_id = node.last_type_id.wrapping_add(id);
            pending.extend(node.children.iter().rev().copied());
            self.by_id.push(index);
        }
        // CVTDQ2PS followed by exponent extraction, including zero -> -126.
        self.type_bits = (((self.by_id.len() as i32 as f32).to_bits() >> 23) & 0xff) as i32 - 126;
        self.by_name.clone_from(&self.registration_order);
        self.initialized = true;
        Ok(true)
    }

    pub fn node(&self, index: usize) -> Option<&ClassNode> {
        self.nodes.get(index)
    }
    pub fn roots(&self) -> &[usize] {
        &self.roots
    }
    pub fn children(&self, index: usize) -> Option<&[usize]> {
        Some(&self.nodes.get(index)?.children)
    }
    pub fn by_name(&self) -> &[usize] {
        &self.by_name
    }
    pub fn by_id(&self) -> &[usize] {
        &self.by_id
    }
    pub fn type_bits(&self) -> i32 {
        self.type_bits
    }
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
    pub fn type_id(&self, index: usize) -> Result<i32, String> {
        if !self.initialized {
            return Err("class hierarchy is not initialized".into());
        }
        Ok(self
            .nodes
            .get(index)
            .ok_or("class node is missing")?
            .type_id)
    }
    pub fn type_range(&self, index: usize) -> Result<[i32;2], String> {
        let first = self.type_id(index)?;
        Ok([first,self.nodes[index].last_type_id])
    }
}

#[cfg(test)]
#[path = "class_hierarchy/tests.rs"]
mod tests;
