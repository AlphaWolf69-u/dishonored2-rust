//! cbc5d0's convex support inputs, before the actual closest-point kernel.

#[derive(Clone, Copy, Debug)]
pub struct ConvexSupportScale {
    pub scale: [f32; 3],
    pub offset: [f32; 3],
    pub radius: f32,
}

#[derive(Clone, Copy)]
pub struct ConvexSupportInput<'a> {
    pub shape_type: u8,
    pub vertices: &'a [[f32; 4]],
    pub radius: f32,
    pub scaled: Option<ConvexSupportScale>,
}

pub struct ConvexSupport<'a> {
    pub kind: u8,
    pub vertices: &'a [[f32; 4]],
    pub radius: f32,
}

/// A query's two inputs require independent workspaces while both are borrowed.
/// Unscaled input borrows the actual shape data without copying it.
#[derive(Default)]
pub struct ConvexSupportWorkspace {
    scaled: Vec<[f32; 4]>,
}

impl ConvexSupportWorkspace {
    pub fn prepare<'a>(
        &'a mut self,
        input: ConvexSupportInput<'a>,
    ) -> Result<ConvexSupport<'a>, String> {
        // 1423eeda9[shape_type*2]. Other shape types route through their own
        // handlers; do not send their table value8 into an8x8 convex kernel.
        let kind = *[4, 5, 0, 2, 3, 5, 5]
            .get(usize::from(input.shape_type))
            .ok_or("non-convex shape routed to convex support preparation")?;
        if input.vertices.len() > usize::from(u16::MAX) {
            return Err("convex support vertex count exceeds source16-bit field".into());
        }
        let (vertices, radius) = if let Some(scale) = input.scaled {
            self.scaled.clear();
            self.scaled.extend(input.vertices.iter().map(|vertex| {
                let mut result = *vertex;
                for axis in 0..3 {
                    result[axis] = scale.scale[axis] * vertex[axis] + scale.offset[axis];
                }
                result
            }));
            (self.scaled.as_slice(), scale.radius)
        } else {
            (input.vertices, input.radius)
        };
        Ok(ConvexSupport {
            kind,
            vertices,
            radius,
        })
    }
}

#[cfg(test)]
#[path = "convex_support/tests.rs"]
mod tests;
