use super::*;

fn normalized(pool: &NamePool, key: Option<NameId>) -> [u32; 2] {
    let Some(key) = key else {
        return [u32::MAX, 0];
    };
    if key == NameId::empty() {
        return [u32::MAX, 1];
    }
    for (index, block) in pool.blocks.iter().enumerate() {
        if let Some(offset) = key
            .identity()
            .checked_sub(block.address())
            .filter(|&v| v < block.allocation_size)
        {
            return [index as u32, offset as u32];
        }
    }
    panic!("name key is outside owned blocks");
}
fn snapshot(pool: &NamePool, key: NameId, bucket: usize) -> Vec<u32> {
    let mut out = normalized(pool, Some(key)).to_vec();
    out.extend([pool.cache_hash, u32::from(pool.cache_kind)]);
    out.extend(normalized(
        pool,
        pool.cache_entry.map(|entry| pool.key(entry)),
    ));
    out.push(pool.blocks.len() as u32);
    for block in &pool.blocks {
        out.extend([block.allocation_size as u32, block.used as u32]);
    }
    let mut chain = Vec::new();
    let mut at = pool.buckets[bucket];
    while at != NONE {
        chain.extend(normalized(pool, Some(pool.key(at))));
        at = pool.entries[at as usize].next;
    }
    out.push(chain.len() as u32 / 2);
    out.extend(chain);
    out
}

#[test]
fn names_match_complete_executed_cache_lookup_and_block_allocation() {
    let mut pool = NamePool::default();
    let mut retained = Vec::new();
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/name_pool.txt").lines() {
        let mut parts = line.split_whitespace();
        if parts.next().unwrap() == "reset" {
            pool = NamePool::default();
            retained.clear();
            continue;
        }
        let mut words = parts.map(|v| u32::from_str_radix(v, 16).unwrap());
        let kind = words.next().unwrap();
        let n = words.next().unwrap() as usize;
        let seed = words.next().unwrap() as usize;
        let zero = words.next().unwrap();
        let mut data: Vec<_> = if kind == 2 {
            (0..n).map(|_| words.next().unwrap() as u8).collect()
        } else {
            (0..n).map(|i| ((i * 17 + seed) % 255 + 1) as u8).collect()
        };
        if zero != u32::MAX {
            data[zero as usize] = 0;
        }
        let key = pool.intern((kind != 0).then_some(data.as_slice())).unwrap();
        let text = &data[..data.iter().position(|&v| v == 0).unwrap_or(data.len())];
        assert_eq!(pool.text(key).unwrap(), text);
        let bucket = (declaration_name_hash(text) & 0xffff) as usize;
        assert_eq!(
            snapshot(&pool, key, bucket),
            words.collect::<Vec<_>>(),
            "case {cases}"
        );
        retained.push((key, text.to_vec()));
        // New blocks and growing owned entry arrays must not invalidate names.
        for (key, text) in &retained {
            assert_eq!(pool.text(*key), Some(text.as_slice()));
        }
        cases += 1;
    }
    assert_eq!(cases, 512);
}
