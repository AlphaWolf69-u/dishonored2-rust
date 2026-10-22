//! Four-image environment override publication from 88e220. Fog curves and
//! display colour cubes use the same identity grouping and weight preparation.

/// One frame's ordered explicit overrides followed by weighted environments.
/// Empty and zero-weight slots are distinct: a zero-weight image still occupies
/// a slot and can prevent a fifth distinct image from being admitted.
pub struct ImageOverrides<T> {
    images: [Option<T>; 4],
    weights: [f32; 4],
    influences: [f32; 4],
}

pub struct ImageOverrideBlend<T> {
    pub images: [Option<T>; 4],
    pub weights: [f32; 4],
    pub total_weight: f32,
}
impl<T> ImageOverrideBlend<T> {
    /// The fog manager publishes mode 2 only above this ordered threshold;
    /// equal and unordered totals retain mode 1.
    pub fn is_active(&self) -> bool {
        self.total_weight > f32::from_bits(0x3586_37bd)
    }
}
impl<T> Default for ImageOverrides<T> {
    fn default() -> Self {
        Self {
            images: std::array::from_fn(|_| None),
            weights: [0.; 4],
            influences: [0.; 4],
        }
    }
}
impl<T: PartialEq> ImageOverrides<T> {
    fn slot(&mut self, image: Option<T>) -> Option<usize> {
        let image = image?;
        let index = self
            .images
            .iter()
            .position(|stored| stored.as_ref().is_none_or(|stored| *stored == image))?;
        self.images[index] = Some(image);
        Some(index)
    }

    /// The ordered explicit parameter list replaces both factors. Repeating
    /// an image replaces its earlier entry; its final contribution is weight²
    /// unless a subsequent environment raises one of the factors.
    pub fn set_override(&mut self, image: Option<T>, weight: f32) {
        if let Some(index) = self.slot(image) {
            self.weights[index] = weight;
            self.influences[index] = weight;
        }
    }

    /// Environment entries raise each factor independently. MAXSS selects the
    /// incoming operand for equal or unordered inputs; f32::max differs.
    pub fn add_environment(&mut self, image: Option<T>, weight: f32, influence: f32) {
        if let Some(index) = self.slot(image) {
            self.weights[index] = if self.weights[index] > weight {
                self.weights[index]
            } else {
                weight
            };
            self.influences[index] = if self.influences[index] > influence {
                self.influences[index]
            } else {
                influence
            };
        }
    }

    pub fn finish(self) -> ImageOverrideBlend<T> {
        let mut weights = std::array::from_fn(|i| self.weights[i] * self.influences[i]);
        let total = |v: [f32; 4]| (((v[0] + 0.) + v[1]) + v[2]) + v[3];
        let mut sum = total(weights);
        if sum > 1. {
            let reciprocal = 1. / sum;
            for weight in &mut weights {
                *weight *= reciprocal;
            }
            sum = total(weights);
        }
        ImageOverrideBlend {
            images: self.images,
            weights,
            total_weight: sum,
        }
    }
}

#[cfg(test)]
#[path = "environment_images/tests.rs"]
mod tests;
