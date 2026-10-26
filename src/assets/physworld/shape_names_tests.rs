#[test]
fn shape_extensions_match_executed_extraction() {
    let decode = |text: &str| {
        if text == "-" {
            return String::new();
        }
        String::from_utf8(
            text.as_bytes()
                .chunks_exact(2)
                .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
                .collect(),
        )
        .unwrap()
    };
    for line in include_str!("../../../tests/fixtures/shape_extension.txt").lines() {
        let (name, expected) = line.split_once(' ').unwrap();
        assert_eq!(
            super::shape_extension(&decode(name)),
            decode(expected),
            "{line}"
        );
    }
}
