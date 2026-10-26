/// Midpoint-pivot quicksort from 1290410 (16-byte body records) and d92ea0
/// (8-byte leaf/index pairs). Standard unstable sorts need not preserve these
/// equal-key swaps, which affect the ordered contact query.
pub(super) fn by_key<T>(values: &mut [T], key: impl Fn(&T) -> u32) {
    fn sort<T>(values: &mut [T], key: &impl Fn(&T) -> u32, mut lo: isize, hi: isize) {
        loop {
            let (mut i, mut j) = (lo, hi);
            let pivot = key(&values[((lo + hi) >> 1) as usize]);
            loop {
                while key(&values[i as usize]) < pivot {
                    i += 1;
                }
                while pivot < key(&values[j as usize]) {
                    j -= 1;
                }
                if j < i {
                    break;
                }
                if i != j {
                    values.swap(i as usize, j as usize);
                }
                i += 1;
                j -= 1;
                if i > j {
                    break;
                }
            }
            if lo < j {
                sort(values, key, lo, j);
            }
            if hi <= i {
                return;
            }
            lo = i;
        }
    }
    if values.len() > 1 {
        sort(values, &key, 0, values.len() as isize - 1);
    }
}
