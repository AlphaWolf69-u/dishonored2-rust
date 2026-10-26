use super::*;

#[test]
fn resizing_matches_executed_fill_and_padding_rules() {
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/bit_field_resize.txt").lines() {
        let mut values = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let bits = values.next().unwrap() as usize;
        let n = values.next().unwrap() as usize;
        let words = values.by_ref().take(n).collect();
        let mut field = BitField { words, bits };
        let new = values.next().unwrap() as usize;
        let fill = values.next().unwrap() as i32;
        let n = values.next().unwrap() as usize;
        let expected: Vec<_> = values.collect();
        assert_eq!(expected.len(), n);
        field.resize(new, fill);
        assert_eq!(field.words(), expected, "case {count}");
        assert_eq!(field.len(), new);
        count += 1;
    }
    assert_eq!(count, 1024);
}
