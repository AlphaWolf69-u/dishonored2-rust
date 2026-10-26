//! Initialization dependency expansion and grouped traversal7e9dc0/7e8c90/7ec0b0.
/// Class identity and the already materialized inclusive RTTI range.
#[derive(Clone, Copy, Debug)]
pub struct DependencyClass {
    pub identity: usize,
    pub type_range: [i32; 2],
}
#[derive(Clone, Debug)]
pub struct Dependency {
    pub class: DependencyClass,
    pub exact: bool,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DependencyError {
    SelfDependency { component: usize },
    Cycle { path: Vec<usize> },
}
pub struct InitializationPlan {
    /// Weakly connected groups in source discovery order, each then reordered
    /// by dependency-first postorder. No generic topological-sort tie breaker.
    pub groups: Vec<Vec<usize>>,
    pub ranks: Vec<i32>,
}
struct Node {
    dependencies: Vec<usize>,
    dependents: Vec<usize>,
    flags: u8,
}
impl InitializationPlan {
    pub fn build(types: &[(DependencyClass, Vec<Dependency>)]) -> Result<Self, DependencyError> {
        let mut nodes: Vec<_> = types
            .iter()
            .map(|_| Node {
                dependencies: Vec::new(),
                dependents: Vec::new(),
                flags: 0,
            })
            .collect();
        //7e9dc0 visits declarations first, then every type in factory order.
        // Duplicate declarations and expanded targets remain duplicate edges.
        for (component, (_, dependencies)) in types.iter().enumerate() {
            for dependency in dependencies {
                for (candidate, (class, _)) in types.iter().enumerate() {
                    let accepted = if dependency.exact {
                        dependency.class.identity == class.identity
                    } else {
                        (dependency.class.type_range[0]..=dependency.class.type_range[1])
                            .contains(&class.type_range[0])
                    };
                    if accepted {
                        if component == candidate {
                            return Err(DependencyError::SelfDependency { component });
                        }
                        nodes[component].dependencies.push(candidate);
                        nodes[candidate].dependents.push(component);
                    }
                }
            }
        }
        fn gather(at: usize, nodes: &mut [Node], group: &mut Vec<usize>) {
            if nodes[at].flags & 1 != 0 {
                return;
            }
            nodes[at].flags |= 1;
            group.push(at);
            for edge in 0..nodes[at].dependencies.len() {
                gather(nodes[at].dependencies[edge], nodes, group);
            }
            for edge in 0..nodes[at].dependents.len() {
                gather(nodes[at].dependents[edge], nodes, group);
            }
        }
        let mut groups = Vec::new();
        for at in 0..nodes.len() {
            if nodes[at].flags & 1 == 0 {
                let mut group = Vec::new();
                gather(at, &mut nodes, &mut group);
                groups.push(group);
            }
        }
        fn visit(at: usize, nodes: &mut [Node], sorted: &mut Vec<usize>) -> Result<(), Vec<usize>> {
            if nodes[at].flags & 2 != 0 {
                return Err(vec![at]);
            }
            if nodes[at].flags & 4 != 0 {
                return Ok(());
            }
            nodes[at].flags |= 2;
            for edge in 0..nodes[at].dependencies.len() {
                if let Err(mut path) = visit(nodes[at].dependencies[edge], nodes, sorted) {
                    path.insert(0, at);
                    return Err(path);
                }
            }
            nodes[at].flags = (nodes[at].flags & !2) | 4;
            sorted.push(at);
            Ok(())
        }
        for group in &mut groups {
            let mut sorted = Vec::with_capacity(group.len());
            for &at in group.iter() {
                visit(at, &mut nodes, &mut sorted)
                    .map_err(|path| DependencyError::Cycle { path })?;
            }
            *group = sorted;
        }
        let mut rank = types.len() as i32;
        let mut ranks = vec![0; types.len()];
        for &component in groups.iter().flatten() {
            rank -= 1;
            ranks[component] = rank;
        }
        Ok(Self { groups, ranks })
    }
}
#[cfg(test)]
mod tests;
