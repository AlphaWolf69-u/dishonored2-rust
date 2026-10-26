use super::*;

#[test]
fn slot_lifecycle_matches_executed_query_manager() {
    let mut pool = CollisionQueries::<u32, u32>::default();
    let mut handles = vec![QueryId::INVALID; QUERY_CAPACITY + 1];
    let mut output = Vec::with_capacity(8);
    output.push(0x1234_5678);
    let mut execution_hash = 2_166_136_261u32;
    let mut count = 0;
    for (line, row) in include_str!("../../../tests/fixtures/collision_query_lifecycle.txt")
        .lines()
        .enumerate()
    {
        let row: Vec<_> = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let op = row[0];
        let h = row[1] as usize;
        let argument = row[2];
        let watch = row[3] as usize;
        let before = handles[h];
        let pointer = output.as_ptr();
        let result = match op {
            0 | 4 => {
                handles[h] = pool.submit(argument, op == 0).unwrap();
                u32::from(handles[h].is_valid())
            }
            1 => {
                pool.cancel(&mut handles[h]).unwrap();
                1
            }
            2 => u32::from(pool.take_completed(&mut handles[h], &mut output).unwrap()),
            3 => {
                pool.resolve_pending(|&token, hits| {
                    execution_hash = (execution_hash ^ token).wrapping_mul(16_777_619);
                    for i in 0..(token & 3) {
                        let i = if token & 16 != 0 { 0 } else { i };
                        hits.push(token.wrapping_mul(17).wrapping_add(i * 3 + 7));
                    }
                });
                1
            }
            // Simulate the same free slot after many generations, just before
            // its32-bit counter wraps; this is fixture setup, not a query API.
            5 => {
                pool.slots[watch].id = QueryId(argument);
                1
            }
            other => panic!("unknown lifecycle operation {other}"),
        };
        assert_eq!(result, row[4], "return case {line}");
        assert_eq!(handles[h].bits(), row[5], "caller id case {line}");
        assert_eq!(pool.free_head, row[6], "free chain case {line}");
        assert_eq!(
            pool.pending.len(),
            row[7] as usize,
            "pending count case {line}"
        );
        assert_eq!(
            pool.slots[watch].id.bits(),
            row[8],
            "slot generation case {line}"
        );
        assert_eq!(
            pool.slots[watch].flags, row[9] as u8,
            "slot state case {line}"
        );
        assert_eq!(
            u32::from(pointer != output.as_ptr()),
            row[10],
            "result ownership case {line}"
        );
        assert_eq!(output.len(), row[11] as usize, "result count case {line}");
        assert_eq!(execution_hash, row[12], "FIFO executor order case {line}");
        assert_eq!(&output, &row[13..], "ordered hit payload case {line}");
        if op == 2 && result != 0 {
            assert_eq!(pool.index(before), Err(QueryError::OutdatedId));
        }
        count += 1;
    }
    assert_eq!(count, 12_222);
}

#[test]
fn submission_phase_errors_preserve_slot_ownership() {
    let mut pool = CollisionQueries::<(), u32>::default();
    pool.set_submission_allowed(false);
    assert_eq!(pool.submit((), false), Ok(QueryId::INVALID));
    assert_eq!(
        pool.submit((), true),
        Err(QueryError::OutsideSubmissionPhase)
    );
    pool.set_submission_allowed(true);
    let id = pool.submit((), true).unwrap();
    assert_eq!(id.bits(), 0x2000);
    assert_eq!(pool.pending.len(), 1);
}

#[test]
fn geometry_failure_does_not_complete_or_publish_partial_hits() {
    let mut pool = CollisionQueries::<u32, u32>::default();
    let mut first = pool.submit(1, true).unwrap();
    let mut failed = pool.submit(2, true).unwrap();
    let mut later = pool.submit(3, true).unwrap();
    let result = pool.try_resolve_pending(|&request, hits| {
        hits.push(request * 10);
        if request == 2 {
            Err("missing shape")
        } else {
            Ok(())
        }
    });
    assert_eq!(result, Err("missing shape"));
    let mut output = vec![77];
    assert!(pool.take_completed(&mut first, &mut output).unwrap());
    assert_eq!(output, [10]);
    assert!(!pool.take_completed(&mut failed, &mut output).unwrap());
    assert!(!pool.take_completed(&mut later, &mut output).unwrap());
    assert_eq!(output, [10]);
    let mut order = Vec::new();
    pool.try_resolve_pending(|&request, hits| {
        order.push(request);
        hits.push(request * 100);
        Ok::<_, ()>(())
    })
    .unwrap();
    assert_eq!(order, [2, 3]);
    assert!(pool.take_completed(&mut failed, &mut output).unwrap());
    assert_eq!(output, [200]);
    assert!(pool.take_completed(&mut later, &mut output).unwrap());
    assert_eq!(output, [300]);
}
