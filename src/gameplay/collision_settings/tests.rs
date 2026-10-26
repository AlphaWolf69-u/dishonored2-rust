use super::*;
#[test]
fn collision_settings_match_executed_constructors_and_upgrade() {
    let mut count = 0;
    for (case, line) in include_str!("../../../tests/fixtures/collision_settings.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<u32> = line
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        let mut settings = CollisionSettings {
            flags: w[1] as u16,
            version: w[2] as i32,
            blocker: w[3],
            legacy_flags: w[4] as u8,
        };
        match w[0] {
            0 => settings = CollisionSettings::default(),
            1 => {
                settings = CollisionSettings::from_blocking(std::array::from_fn(|bit| {
                    w[5] & (1 << bit) != 0
                }))
            }
            2 => settings.upgrade(),
            _ => unreachable!(),
        }
        assert_eq!(
            [
                u32::from(settings.flags),
                settings.version as u32,
                settings.blocker,
                u32::from(settings.legacy_flags)
            ],
            w[6..],
            "case {case}"
        );
        count += 1;
    }
    assert_eq!(count, 769);
}
