//! The character proxy's simplex solver (FUN_140b82be0, hkSimplexSolver):
//! moves a point through time against surface constraint planes, keeping at
//! most four planes in support and resolving their velocity with the 1/2/3
//! plane solvers FUN_140b83180 / FUN_140b834b0 / FUN_140b838f0.
use bevy_math::Vec3;

/// 1.1920929e-7 (0x1423cce30).
const EPSILON: f32 = f32::EPSILON;

/// hkSurfaceConstraintInfo (0x40 bytes): plane `normal`/`distance`, surface
/// velocity, frictions and priority.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceConstraint {
    pub normal: Vec3,
    pub distance: f32,
    pub velocity: Vec3,
    pub static_friction: f32,
    pub extra_up_static_friction: f32,
    pub extra_down_static_friction: f32,
    pub dynamic_friction: f32,
    pub priority: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlaneInteraction {
    pub touched: bool,
    pub surface_time: f32,
    pub penalty_distance: f32,
    /// 0 OK, 1 three-plane failure, 2 two-plane failure.
    pub status: i32,
}

pub struct SolverInput<'a> {
    pub position: Vec3,
    pub velocity: Vec3,
    pub max_surface_velocity: Vec3,
    pub up: Vec3,
    pub delta_time: f32,
    pub min_delta_time: f32,
    pub constraints: &'a [SurfaceConstraint],
}

#[derive(Clone, Debug, PartialEq)]
pub struct SolverOutput {
    pub position: Vec3,
    pub velocity: Vec3,
    pub delta_time: f32,
    pub interactions: Vec<PlaneInteraction>,
}

struct Solver<'a> {
    input: &'a SolverInput<'a>,
    output: SolverOutput,
    support: Vec<usize>,
}

pub fn solve(input: &SolverInput) -> SolverOutput {
    let mut solver = Solver {
        input,
        output: SolverOutput {
            position: input.position,
            velocity: input.velocity,
            delta_time: 0.0,
            interactions: vec![PlaneInteraction::default(); input.constraints.len()],
        },
        support: Vec::with_capacity(4),
    };
    solver.run();
    solver.output
}

impl Solver<'_> {
    fn constraint(&self, index: usize) -> &SurfaceConstraint {
        &self.input.constraints[index]
    }

    fn run(&mut self) {
        let mut remaining = self.input.delta_time;
        let mut current_time = 0.0f32;
        if remaining < 0.0 {
            return;
        }
        loop {
            let mut min_time = remaining;
            let mut hit = None;
            for index in 0..self.input.constraints.len() {
                if self.support.contains(&index) || self.output.interactions[index].status != 0 {
                    continue;
                }
                let constraint = self.constraint(index);
                let approach = -(self.output.velocity - constraint.velocity).dot(constraint.normal);
                if approach > 0.0 {
                    let mut distance = (self.output.position - constraint.velocity * current_time)
                        .dot(constraint.normal)
                        + constraint.distance;
                    if distance <= EPSILON {
                        distance = 0.0;
                    }
                    distance += self.output.interactions[index].penalty_distance;
                    if distance < min_time * approach {
                        min_time = distance / approach;
                        hit = Some(index);
                    }
                }
            }
            // 1e-4 (DAT_141ab3b00).
            if min_time > 1e-4 {
                current_time += min_time;
                remaining -= min_time;
                self.output.position += self.output.velocity * min_time;
                for &index in &self.support {
                    let interaction = &mut self.output.interactions[index];
                    interaction.surface_time += min_time;
                    interaction.touched = true;
                }
                self.output.delta_time = current_time;
                if current_time > self.input.min_delta_time {
                    return;
                }
            }
            let Some(hit) = hit else { break };
            self.support.push(hit);
            let interaction = &mut self.output.interactions[hit];
            // 2.0 (DAT_141ab3b20).
            interaction.penalty_distance = (interaction.penalty_distance + EPSILON) * 2.0;
            self.examine_active_planes();
            if remaining < 0.0 {
                return;
            }
        }
        self.output.delta_time = self.input.delta_time;
    }

    /// FUN_140b83120: -0.001 (DAT_141ae87bc).
    fn violates(&self, index: usize, velocity: Vec3) -> bool {
        let constraint = self.constraint(index);
        (velocity - constraint.velocity).dot(constraint.normal) < -0.001
    }

    /// FUN_140b83bc0.
    fn examine_active_planes(&mut self) {
        let input_velocity = self.input.velocity;
        loop {
            match self.support.len() {
                1 => {
                    self.output.velocity = self.solve1(self.support[0], input_velocity);
                    return;
                }
                2 => {
                    let candidate = self.solve1(self.support[1], input_velocity);
                    if !self.violates(self.support[0], candidate) {
                        self.output.velocity = candidate;
                        self.support = vec![self.support[1]];
                        return;
                    }
                    self.output.velocity = self.solve2(self.support[0], self.support[1], input_velocity);
                    return;
                }
                3 => {
                    let s = [self.support[0], self.support[1], self.support[2]];
                    let candidate = self.solve1(s[2], input_velocity);
                    if !self.violates(s[0], candidate) && !self.violates(s[1], candidate) {
                        self.output.velocity = candidate;
                        self.support = vec![s[2]];
                        continue;
                    }
                    let mut kept = None;
                    for k in 0..2 {
                        let candidate = self.solve2(s[k], s[2], input_velocity);
                        if !self.violates(s[1 - k], candidate) {
                            kept = Some(k);
                            break;
                        }
                    }
                    if let Some(k) = kept {
                        self.support = vec![s[k], s[2]];
                        continue;
                    }
                    self.output.velocity = self.solve3(s[0], s[1], s[2], true, input_velocity);
                    return;
                }
                4 => {
                    self.sort_support();
                    let mut s = [self.support[0], self.support[1], self.support[2], self.support[3]];
                    let mut dropped = false;
                    for i in 1..=3 {
                        let k = i - 1;
                        let candidate = self.solve3(s[i % 3], s[(i + 1) % 3], s[3], false, input_velocity);
                        if !self.violates(s[k], candidate) {
                            s[k] = s[2];
                            s[2] = s[3];
                            self.support = vec![s[0], s[1], s[2]];
                            dropped = true;
                            break;
                        }
                    }
                    if dropped {
                        continue;
                    }
                    let mut velocity = input_velocity;
                    velocity = self.solve3(s[0], s[1], s[2], false, velocity);
                    velocity = self.solve3(s[0], s[1], s[3], false, velocity);
                    velocity = self.solve3(s[0], s[2], s[3], false, velocity);
                    velocity = self.solve3(s[1], s[2], s[3], false, velocity);
                    self.output.velocity = velocity;
                    let worst = s.iter().map(|&i| self.output.interactions[i].status).max().unwrap_or(0).max(0);
                    let k = s.iter().position(|&i| self.output.interactions[i].status == worst).unwrap_or(0);
                    s[k] = s[3];
                    self.support = vec![s[0], s[1], s[2]];
                    for &index in &self.support {
                        self.output.interactions[index].status = 0;
                    }
                }
                _ => return,
            }
        }
    }

    /// FUN_140b83030: ascending priority, ties by ascending surface speed.
    fn sort_support(&mut self) {
        let n = self.support.len();
        for i in 0..n {
            for j in i + 1..n {
                let (a, b) = (self.constraint(self.support[i]), self.constraint(self.support[j]));
                if b.priority < a.priority
                    || (b.priority == a.priority && b.velocity.length_squared() <= a.velocity.length_squared())
                {
                    self.support.swap(i, j);
                }
            }
        }
    }

    /// FUN_140b83180.
    fn solve1(&self, index: usize, velocity: Vec3) -> Vec3 {
        let c = self.constraint(index);
        let up = self.input.up;
        let relative = velocity - c.velocity;
        let d = relative.dot(c.normal);
        let relative_length2 = relative.length_squared();
        let mut tangent = relative - c.normal * d;
        let static_friction = c.static_friction;
        let extra = if up.dot(tangent) <= 0.0 {
            c.extra_down_static_friction
        } else {
            c.extra_up_static_friction
        };
        if extra <= 0.0 {
            if relative_length2 <= (static_friction * static_friction + 1.0) * d * d {
                return c.velocity;
            }
        } else {
            let mut horizontal = up.cross(c.normal);
            let mut horizontal_dot = 0.0;
            if horizontal.length_squared() > EPSILON {
                horizontal = horizontal.normalize();
                horizontal_dot = horizontal.dot(tangent);
                if horizontal_dot * horizontal_dot <= static_friction * static_friction * d * d {
                    tangent -= horizontal * horizontal_dot;
                    horizontal_dot = 0.0;
                }
            }
            if relative_length2 - horizontal_dot * horizontal_dot - d * d
                <= (extra + static_friction) * (extra + static_friction) * d * d
            {
                if horizontal_dot == 0.0 {
                    return c.velocity;
                }
                tangent = horizontal * horizontal_dot;
            }
        }
        let dynamic_friction = c.dynamic_friction;
        let tangent_length2 = tangent.length_squared();
        // 0.001 (DAT_141ab3b04).
        if dynamic_friction < 1.0 && tangent_length2 >= EPSILON && 0.001 * relative_length2 < tangent_length2 {
            tangent *= (1.0 - dynamic_friction) * (relative_length2 / tangent_length2).sqrt() + dynamic_friction;
            tangent -= c.normal * tangent.dot(c.normal);
        }
        tangent + c.velocity
    }

    /// FUN_140b834b0.
    fn solve2(&mut self, first: usize, second: usize, velocity: Vec3) -> Vec3 {
        let (c1, c2) = (*self.constraint(first), *self.constraint(second));
        let up = self.input.up;
        let crease = c1.normal.cross(c2.normal);
        let crease_length2 = crease.length_squared();
        if crease_length2 > EPSILON {
            let crease_length = crease_length2.sqrt();
            let axis = crease / crease_length;
            let inverse_length = 1.0 / crease_length;
            // 0.5 (0x1423ccd20).
            let axis_velocity = (c1.velocity + c2.velocity).dot(axis) * 0.5;
            let plane_velocity = (c2.normal.cross(axis) * c1.normal.dot(c1.velocity)
                + axis.cross(c1.normal) * c2.normal.dot(c2.velocity))
                * inverse_length
                + axis * axis_velocity;
            if plane_velocity.abs().cmple(self.input.max_surface_velocity).all() {
                let relative = velocity - plane_velocity;
                let relative_length2 = relative.length_squared();
                let mut along = relative.dot(axis);
                let up_along = up.dot(axis);
                let extra = if along * up_along <= 0.0 {
                    c1.extra_down_static_friction + c2.extra_down_static_friction
                } else {
                    c1.extra_up_static_friction + c2.extra_up_static_friction
                };
                let static_friction = (c1.static_friction + c2.static_friction + extra * up_along) * 0.5;
                if along * along <= static_friction * static_friction * (relative_length2 - along * along) {
                    return plane_velocity;
                }
                let dynamic_friction = (c1.dynamic_friction + c2.dynamic_friction) * 0.5;
                if dynamic_friction < 1.0 && 0.001 * relative_length2 < along * along {
                    along *= relative_length2.sqrt() * (1.0 / along).abs() * (1.0 - dynamic_friction) + dynamic_friction;
                }
                return axis * along + plane_velocity;
            }
        }
        self.output.interactions[first].status = 2;
        self.output.interactions[second].status = 2;
        if c1.priority > c2.priority {
            let velocity = self.solve1(second, velocity);
            self.solve1(first, velocity)
        } else {
            let velocity = self.solve1(first, velocity);
            self.solve1(second, velocity)
        }
    }

    /// FUN_140b838f0.
    fn solve3(&mut self, first: usize, second: usize, third: usize, sort: bool, velocity: Vec3) -> Vec3 {
        let (c1, c2, c3) = (*self.constraint(first), *self.constraint(second), *self.constraint(third));
        let determinant = c1.normal.dot(c2.normal.cross(c3.normal));
        if determinant.abs() >= EPSILON {
            let point_velocity = (c2.normal.cross(c3.normal) * c1.normal.dot(c1.velocity)
                + c3.normal.cross(c1.normal) * c2.normal.dot(c2.velocity)
                + c1.normal.cross(c2.normal) * c3.normal.dot(c3.velocity))
                / determinant;
            if point_velocity.abs().cmple(self.input.max_surface_velocity).all() {
                return point_velocity;
            }
        }
        let (mut a, mut b, mut c) = (first, second, third);
        if sort {
            self.sort_support();
            a = self.support[0];
            b = self.support[1];
            c = self.support[2];
        }
        for index in [a, b, c] {
            self.output.interactions[index].status = 1;
        }
        let velocity = self.solve2(a, b, velocity);
        let velocity = self.solve2(a, c, velocity);
        self.solve2(b, c, velocity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane(normal: Vec3, distance: f32) -> SurfaceConstraint {
        SurfaceConstraint {
            normal: normal.normalize(),
            distance,
            velocity: Vec3::ZERO,
            static_friction: 0.0,
            extra_up_static_friction: 0.0,
            extra_down_static_friction: 0.0,
            dynamic_friction: 1.0,
            priority: 2,
        }
    }

    fn run(velocity: Vec3, constraints: &[SurfaceConstraint]) -> SolverOutput {
        solve(&SolverInput {
            position: Vec3::ZERO,
            velocity,
            max_surface_velocity: Vec3::splat(10.0),
            up: Vec3::Z,
            delta_time: 1.0 / 60.0,
            min_delta_time: 0.0,
            constraints,
        })
    }

    #[test]
    fn free_motion_keeps_velocity_for_the_whole_step() {
        let output = run(Vec3::new(1.0, 0.0, 0.0), &[]);
        assert_eq!(output.velocity, Vec3::new(1.0, 0.0, 0.0));
        assert!((output.position.x - 1.0 / 60.0).abs() < 1e-6);
    }

    #[test]
    fn floor_contact_projects_velocity_onto_the_plane() {
        let output = run(Vec3::new(3.0, 0.0, -5.0), &[plane(Vec3::Z, 0.0)]);
        assert!(output.velocity.distance(Vec3::new(3.0, 0.0, 0.0)) < 1e-5, "{output:?}");
        assert!(output.interactions[0].touched);
    }

    #[test]
    fn two_walls_leave_motion_along_their_crease() {
        let output = run(
            Vec3::new(-2.0, -2.0, 1.0),
            &[plane(Vec3::X, 0.0), plane(Vec3::Y, 0.0)],
        );
        assert!(output.velocity.distance(Vec3::new(0.0, 0.0, 1.0)) < 1e-5, "{output:?}");
    }

    #[test]
    fn three_planes_meeting_at_a_corner_stop_the_point() {
        let output = run(
            Vec3::new(-1.0, -1.0, -1.0),
            &[plane(Vec3::X, 0.0), plane(Vec3::Y, 0.0), plane(Vec3::Z, 0.0)],
        );
        assert!(output.velocity.length() < 1e-5, "{output:?}");
    }
}
