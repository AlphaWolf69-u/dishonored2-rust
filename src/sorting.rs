//! Shared idLib sort; callers retain the source comparator and tie ordering.
/// `idSort_Quick::Sort` (`FUN_1403949a0`): idLib's quicksort with an
/// explicit stack and a selection sort below four elements.
pub(crate) fn quick_sort<T: Copy>(a: &mut [T], cmp: impl Fn(&T, &T) -> i32) {
    if a.len() < 2 {
        return;
    }
    let mut lo_stack = [0i64; 128];
    let mut hi_stack = [0i64; 128];
    let mut depth: i64 = 0;
    let mut lo: i64 = 0;
    let mut hi: i64 = a.len() as i64 - 1;
    let at = |i: i64| i as usize;
    loop {
        while hi - lo > 3 && depth < 0x7f {
            a.swap(at(hi), at((hi + lo) / 2));
            let mut i = lo;
            let mut j = hi - 1;
            loop {
                while cmp(&a[at(i)], &a[at(hi)]) < 0 {
                    i += 1;
                    if i >= j {
                        break;
                    }
                }
                let mut met = false;
                while cmp(&a[at(j)], &a[at(hi)]) > 0 {
                    j -= 1;
                    if j <= i {
                        met = true;
                        break;
                    }
                }
                if met || j <= i {
                    break;
                }
                a.swap(at(i), at(j));
                j -= 1;
                i += 1;
                if i >= j {
                    break;
                }
            }
            let pivot = a[at(hi)];
            while cmp(&a[at(j)], &pivot) >= 0 && lo < j {
                j -= 1;
            }
            while cmp(&a[at(i)], &pivot) <= 0 && i < hi {
                i += 1;
            }
            a[at(hi)] = a[at(i)];
            a[at(i)] = pivot;
            if j - lo < hi - i {
                lo_stack[at(depth)] = i;
                hi_stack[at(depth)] = hi;
                depth += 1;
                hi = j;
            } else {
                lo_stack[at(depth)] = lo;
                hi_stack[at(depth)] = j;
                depth += 1;
                lo = i;
            }
        }
        while lo < hi {
            let mut max = lo;
            for k in lo + 1..=hi {
                if cmp(&a[at(k)], &a[at(max)]) > 0 {
                    max = k;
                }
            }
            a.swap(at(max), at(hi));
            hi -= 1;
        }
        depth -= 1;
        if depth < 0 {
            return;
        }
        lo = lo_stack[at(depth)];
        hi = hi_stack[at(depth)];
    }
}
