//! Bilateral linear row consumer, schema5 in 14129af90.
use super::{ball_socket::SolverVelocities, LinearLockRow};
impl LinearLockRow {
    pub fn solve(self, a: &mut SolverVelocities, b: &mut SolverVelocities) -> f32 {
        let velocity: [f32; 3] = std::array::from_fn(|i| {
            (self.angular_b[i] * b.angular[i] + a.angular[i] * self.angular_a[i])
                + (a.linear[i] - b.linear[i]) * self.axis[i]
        });
        let impulse =
            (self.correction - ((velocity[1] + velocity[0]) + velocity[2])) * self.effective_mass;
        let linear_a = impulse * a.inverse_mass[3];
        let linear_b = impulse * b.inverse_mass[3];
        for i in 0..3 {
            a.angular[i] += impulse * a.inverse_mass[i] * self.angular_a[i];
            b.angular[i] += self.angular_b[i] * impulse * b.inverse_mass[i];
            a.linear[i] += linear_a * self.axis[i];
            b.linear[i] -= linear_b * self.axis[i];
        }
        impulse
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linear_lock_consumers_match_executable() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/linear_lock_solve.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let words: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(words[i]);
            let vec = |at| std::array::from_fn(|i| v(at + i));
            let vector = |at| [v(at), v(at + 1), v(at + 2), 0.];
            let body = |at, initial, mass| SolverVelocities {
                linear: vector(at),
                angular: vector(at + 3),
                initial_linear: vector(initial),
                initial_angular: vector(initial + 3),
                inverse_mass: vec(mass),
            };
            let mut a = body(0, 12, 24);
            let mut b = body(6, 18, 28);
            let row = LinearLockRow {
                axis: std::array::from_fn(|i| v(34 + i)),
                correction: v(37),
                angular_a: std::array::from_fn(|i| v(38 + i)),
                effective_mass: v(41),
                angular_b: std::array::from_fn(|i| v(42 + i)),
                inverse_effective_mass: v(45),
            };
            let impulse = row.solve(&mut a, &mut b);
            for (i, actual) in a.linear[..3]
                .iter()
                .chain(&a.angular[..3])
                .chain(&b.linear[..3])
                .chain(&b.angular[..3])
                .copied()
                .chain([impulse, 0.])
                .enumerate()
            {
                assert!(
                    (actual - v(46 + i)).abs() < 0.000003 * v(46 + i).abs().max(1.),
                    "linear case{count} field{i}: {actual} vs {}",
                    v(46 + i)
                );
            }
            count += 1;
        }
        assert_eq!(count, 256);
    }
}
