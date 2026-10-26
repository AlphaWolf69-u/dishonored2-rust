//! Penetration feature preparation, fallback and compaction (c0ed40).
use super::{
    distance::ConvexDistanceSolution,
    faces::reciprocal_sqrt_estimate,
    penetration::{ConvexSupportPoint, PenetrationSupport},
    seed::{cross, dot, inverse_length, normalize, perpendicular, subtract, PenetrationSeed},
    solver::solve_penetration,
    vertices::support_pair,
};

pub struct ConvexFeatures {
    pub query: [[f32; 4]; 4],
    /// XYZ is already in query coordinates; W retains the target vertex ID.
    pub target: [[f32; 4]; 4],
    pub query_count: usize,
    pub target_count: usize,
}

pub fn query_penetration(
    features: &mut ConvexFeatures,
    support: &PenetrationSupport<'_>,
) -> Result<ConvexDistanceSolution, String> {
    let (a, b) = (features.query_count, features.target_count);
    if !(1..=4).contains(&a) || !(1..=4).contains(&b) {
        return Err("penetration feature counts must be in1..4".into());
    }
    let mut count = 4;
    if b < 3 {
        if a < 3 {
            features.query[a] = features.query[0];
            features.query[a + 1] = features.query[0];
            features.target[b] = features.target[0];
            features.target[b + 1] = features.target[0];
            count = 1;
        } else {
            if a < 4 {
                if b < 2 {
                    count = 3;
                } else {
                    features.query[3] = features.query[0];
                    features.target[3] = features.target[1];
                }
            } else {
                features.target[3] = features.target[0];
            }
            features.target[1] = features.target[0];
            features.target[2] = features.target[0];
        }
    } else {
        if b < 4 {
            if a < 2 {
                count = 3;
            } else {
                features.query[3] = features.query[1];
                features.target[3] = features.target[0];
            }
        } else {
            features.query[3] = features.query[0];
        }
        features.query[1] = features.query[0];
        features.query[2] = features.query[0];
    }
    let mut seed = PenetrationSeed {
        count,
        points: std::array::from_fn(|i| ConvexSupportPoint {
            query: features.query[i],
            target: features.target[i],
            difference: subtract(features.query[i], features.target[i]),
        }),
    };
    let mut local_support = PenetrationSupport {
        query: support.query,
        target: support.target,
        relative: support.relative,
    };
    let result = solve_penetration(
        &mut seed,
        &mut local_support,
        &mut 0,
        [f32::from_bits(0x322b_cc76); 4],
        [f32::from_bits(0x38d1_b717); 4],
    )?;
    features.query = seed.points.map(|p| p.query);
    features.target = seed.points.map(|p| p.target);
    if result.status == 7 {
        features.query_count = 1;
        features.target_count = 1;
        return fallback(features, support);
    }
    features.query_count = compact(&mut features.query);
    features.target_count = compact(&mut features.target);
    if features.query_count + features.target_count > 4 {
        if features.target_count < features.query_count {
            features.query_count = 3;
            features.target_count = 1;
        } else {
            features.target_count = 3;
            features.query_count = 1;
        }
    }
    result
        .solution
        .ok_or("penetration search returned success without a solution".into())
}

fn compact(points: &mut [[f32; 4]; 4]) -> usize {
    let id = |i: usize| points[i][3].to_bits() as u16;
    let mut count = 3;
    if id(1) == id(2) || id(0) == id(2) {
        count -= 1;
    }
    if count > 1 && id(0) == id(1) {
        count -= 1;
        points[0] = points[count];
    }
    count
}

fn fallback(
    features: &mut ConvexFeatures,
    support: &PenetrationSupport<'_>,
) -> Result<ConvexDistanceSolution, String> {
    let negative = -reciprocal_sqrt_estimate(3.);
    let mut directions = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [negative; 4],
    ];
    let mut best = [-f32::from_bits(0x7f7f_ffee); 4];
    let mut query = [0.; 4];
    let mut target = [0.; 4];
    let mut step = 1.;
    // Four initial samples followed by twenty four-direction refinements.
    for refinement in 0..=20 {
        for direction in directions {
            let pair = support_pair(
                support.query,
                support.target,
                direction.map(|v| -v),
                support.relative,
            )?;
            let projection = dot(subtract(pair.query, pair.target_in_query), direction);
            if projection > best[3] {
                best = [direction[0], direction[1], direction[2], projection];
                query = pair.query;
                target = pair.target;
            }
        }
        if refinement == 20 {
            break;
        }
        let tangent = normalize(perpendicular(best));
        let bitangent = cross(best, tangent);
        let delta = tangent.map(|v| v * step);
        let across = bitangent.map(|v| v * step);
        let positive: [f32; 4] = std::array::from_fn(|i| delta[i] + best[i]);
        let inverse = inverse_length(dot(positive, positive));
        directions = [
            positive,
            std::array::from_fn(|i| best[i] - delta[i]),
            std::array::from_fn(|i| across[i] + best[i]),
            std::array::from_fn(|i| best[i] - across[i]),
        ]
        .map(|d| d.map(|v| v * inverse));
        step *= 0.5;
    }
    features.query[0] = query;
    features.target[0] = target;
    Ok(ConvexDistanceSolution {
        status: 2,
        distance: [best[3]; 4],
        normal: best,
        witness: query,
    })
}
