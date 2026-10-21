use super::*;

fn digest(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}
fn record_digest(queue: &OceanQueryQueue) -> u64 {
    digest(queue.records.iter().flat_map(|record| {
        record
            .offset
            .to_le_bytes()
            .into_iter()
            .chain(record.count.to_le_bytes())
    }))
}
fn points(count: usize, added: bool) -> Vec<[f32; 2]> {
    (0..count)
        .map(|i| {
            if added {
                [i as f32 * 0.125 + 1., -(i as f32) * 0.75 - 2.]
            } else {
                [i as f32 * 0.25, (-(i as i32)) as f32 * 0.5]
            }
        })
        .collect()
}

#[test]
fn query_handles_match_executed_registration_result_release_and_reset() {
    let mut cases = 0;
    for line in include_str!("../../../../tests/fixtures/ocean_query_handles.txt").lines() {
        let mut fields = line.split_whitespace();
        let operation = fields.next().unwrap();
        let row = fields
            .map(|value| u64::from_str_radix(value, 16).unwrap())
            .collect::<Vec<_>>();
        let mut queue = OceanQueryQueue::new();
        if operation == "register" {
            assert_eq!(row.len(), 11);
            let ring = row[0] as usize;
            let cursor = row[1] as usize;
            let pending = row[2] as usize;
            let added = row[3] as usize;
            let occupied = row[4] as usize;
            queue.next_slot = cursor;
            queue.pending.extend(points(pending, false));
            for j in 0..occupied {
                queue.records[(cursor + j) & 1023] = QueryRecord {
                    offset: (j * 7) as u16,
                    count: (j + 1) as u16,
                };
            }
            let address = queue.pending.as_ptr();
            let handle = queue.register(ring, &points(added, true)).unwrap();
            assert_eq!([handle.slot as u64, handle.ring as u64], row[5..7]);
            assert_eq!(queue.next_slot as u64, row[7]);
            assert_eq!(queue.pending.len() as u64, row[8]);
            assert_eq!(record_digest(&queue), row[9], "{line}");
            assert_eq!(
                digest(
                    queue
                        .pending
                        .iter()
                        .flatten()
                        .flat_map(|value| value.to_le_bytes())
                ),
                row[10],
                "{line}"
            );
            assert_eq!(queue.pending.as_ptr(), address);
        } else {
            assert_eq!(row.len(), 8);
            let ring = row[0] as u16;
            let slot = row[1] as u16;
            let count = row[2] as u16;
            for j in 0..1024 {
                queue.records[j] = QueryRecord {
                    offset: (j % 900) as u16,
                    count: 1,
                };
            }
            if slot != u16::MAX {
                queue.records[usize::from(slot)] = QueryRecord { offset: 13, count };
            }
            let mut handle = OceanQueryHandle {
                slot,
                ring: if slot == u16::MAX { u16::MAX } else { ring },
            };
            let completed: [Vec<[f32; 4]>; 4] = std::array::from_fn(|r| {
                (0..1024)
                    .map(|i| {
                        std::array::from_fn(|channel| {
                            f32::from_bits((r * 10000 + i * 4 + channel) as u32)
                        })
                    })
                    .collect()
            });
            let mut output = vec![[f32::from_bits(0xcdcdcdcd); 4]; 1024];
            let returned = match operation {
                "take" => u64::from(
                    queue
                        .take(
                            &mut handle,
                            completed.each_ref().map(|v| v.as_slice()),
                            &mut output,
                        )
                        .unwrap()
                        .is_some(),
                ),
                "release" => {
                    queue.release(&mut handle).unwrap();
                    0
                }
                "reset" => {
                    queue.reset_handles();
                    0
                }
                _ => panic!("unknown fixture operation{operation}"),
            };
            assert_eq!(returned, row[3]);
            assert_eq!([handle.slot as u64, handle.ring as u64], row[4..6]);
            assert_eq!(record_digest(&queue), row[6], "{line}");
            assert_eq!(
                digest(
                    output
                        .iter()
                        .flatten()
                        .flat_map(|value| value.to_le_bytes())
                ),
                row[7],
                "{line}"
            );
        }
        cases += 1;
    }
    assert_eq!(cases, 1344);
}
