//! 374e40's per-view matrix history. Rendering supplies its actual frame marker;
//! repeated publications in that frame must not retire the preceding frame.
use super::render_shadow::{light_view, multiply_rows, ShadowMatrix};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewMatrices {
    pub position: [f32; 3],
    pub projection: ShadowMatrix,
    pub view: ShadowMatrix,
    pub view_projection: ShadowMatrix,
}
impl Default for ViewMatrices {
    fn default() -> Self {
        let identity = bevy_math::Mat4::IDENTITY.to_cols_array_2d();
        Self {
            position: [0.0; 3],
            projection: identity,
            view: identity,
            view_projection: identity,
        }
    }
}
impl ViewMatrices {
    /// Basis is the game's forward/left/up, not the host camera's XYZ axes.
    pub fn from_camera(position: [f32; 3], basis: [[f32; 3]; 3], projection: ShadowMatrix) -> Self {
        let view = light_view(position, basis);
        Self {
            position,
            projection,
            view,
            view_projection: multiply_rows(projection, view),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewMatrixFrame {
    pub frame: u32,
    pub current: ViewMatrices,
    pub previous: ViewMatrices,
}

pub struct ViewMatrixHistory {
    current: ViewMatrices,
    previous: ViewMatrices,
    last_frame: u32,
    reset: bool,
}
impl Default for ViewMatrixHistory {
    fn default() -> Self {
        Self {
            current: ViewMatrices::default(),
            previous: ViewMatrices::default(),
            last_frame: u32::MAX,
            reset: true,
        }
    }
}
impl ViewMatrixHistory {
    /// Original view+bcfc flag. Reset is consumed by the next update, including
    /// an update that repeats the current frame marker.
    pub fn request_reset(&mut self) {
        self.reset = true;
    }
    pub fn update(&mut self, frame: u32, current: ViewMatrices) -> ViewMatrixFrame {
        if self.last_frame != frame {
            self.previous = self.current;
            self.last_frame = frame;
        }
        self.current = current;
        if self.reset {
            self.previous = self.current;
            self.reset = false;
        }
        ViewMatrixFrame {
            frame,
            current: self.current,
            previous: self.previous,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_history_matches_executed_camera_updates() {
        let mut history = ViewMatrixHistory::default();
        let mut count = 0;
        let bits = |v: ViewMatrices| {
            v.position
                .into_iter()
                .chain(v.projection.into_iter().flatten())
                .chain(v.view.into_iter().flatten())
                .chain(v.view_projection.into_iter().flatten())
                .map(f32::to_bits)
                .collect::<Vec<_>>()
        };
        for row in include_str!("../../tests/fixtures/view_history.txt").lines() {
            let words = row
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 134);
            let f = |i| f32::from_bits(words[i]);
            if words[1] != 0 {
                history.request_reset();
            }
            let matrices = ViewMatrices::from_camera(
                std::array::from_fn(|i| f(2 + i)),
                std::array::from_fn(|r| std::array::from_fn(|c| f(5 + r * 3 + c))),
                std::array::from_fn(|r| std::array::from_fn(|c| f(14 + r * 4 + c))),
            );
            let frame = history.update(words[0], matrices);
            assert_eq!(bits(frame.current), words[30..81], "current case{count}");
            assert_eq!(bits(frame.previous), words[81..132], "previous case{count}");
            assert_eq!(history.last_frame, words[132]);
            assert_eq!(u32::from(history.reset), words[133]);
            count += 1;
        }
        assert_eq!(count, 96);
    }
}
