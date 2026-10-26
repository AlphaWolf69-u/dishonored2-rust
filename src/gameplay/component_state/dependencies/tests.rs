use super::*;
#[test]
fn complete_type_graphs_match_executed_expansion_groups_and_cycle_paths() {
    let mut outcomes = [0usize; 2];
    for (case, row) in include_str!("../../../../tests/fixtures/component_dependencies.txt")
        .lines()
        .enumerate()
    {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let count = words.next().unwrap() as usize;
        let class = |i: usize| DependencyClass {
            identity: i,
            type_range: [
                i as i32,
                if i % 4 == 0 {
                    (i + 3).min(count - 1) as i32
                } else {
                    i as i32
                },
            ],
        };
        let types: Vec<_> = (0..count)
            .map(|i| {
                let n = words.next().unwrap();
                let dependencies = (0..n)
                    .map(|_| {
                        let target = words.next().unwrap() as usize;
                        let exact = words.next().unwrap() != 0;
                        Dependency {
                            class: class(target),
                            exact,
                            reason: "fixture dependency".into(),
                        }
                    })
                    .collect();
                (class(i), dependencies)
            })
            .collect();
        let success = words.next().unwrap() != 0;
        outcomes[usize::from(success)] += 1;
        let n = words.next().unwrap() as usize;
        let actual = InitializationPlan::build(&types);
        if success {
            let expected: Vec<Vec<usize>> = (0..n)
                .map(|_| {
                    let size = words.next().unwrap() as usize;
                    words.by_ref().take(size).map(|v| v as usize).collect()
                })
                .collect();
            let plan = actual.unwrap_or_else(|e| panic!("case {case}: {e:?}"));
            assert_eq!(plan.groups, expected, "case {case}");
            for (position, &node) in expected.iter().flatten().enumerate() {
                assert_eq!(
                    plan.ranks[node],
                    (count - position - 1) as i32,
                    "case {case}, node {node}"
                );
            }
        } else {
            let expected = words
                .by_ref()
                .take(n)
                .map(|v| v as usize)
                .collect::<Vec<_>>();
            match actual {
                Err(DependencyError::Cycle { path }) => assert_eq!(path, expected, "case {case}"),
                Err(error) => panic!("case {case}: unexpected {error:?}"),
                Ok(_) => panic!("case {case}: cycle was accepted"),
            }
        }
        assert!(words.next().is_none());
    }
    assert!(outcomes.iter().all(|&n| n > 0));
}
