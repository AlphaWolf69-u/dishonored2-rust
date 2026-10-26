use super::{field, scope, vector, Definition, Value};
use crate::gameplay::reference_frame::ReferenceFrame;

/// Entity spawn pose with 874c50/18df90 constructor defaults. Audio uses the
/// owner's origin/basis (140709ee0); trigger shape recipes use this same pose.
pub fn spawn_frame(entity: &Definition) -> Result<ReferenceFrame, String> {
    let edit = scope(field(&entity.fields, "edit")).unwrap_or(&[]);
    let read = |fields: &[(String, Value)], name: &str, default| match field(fields, name) {
        None => Ok(default),
        Some(Value::Scope(values)) => {
            vector(values, default).ok_or_else(|| format!("{}: invalid {name}", entity.id))
        }
        Some(_) => Err(format!("{}: expected vector for {name}", entity.id)),
    };
    let mut frame = ReferenceFrame::default();
    frame.position = read(edit, "spawnPosition", frame.position)?;
    if let Some(matrix) = scope(field(edit, "spawnOrientation"))
        .and_then(|orientation| scope(field(orientation, "mat")))
    {
        for (index, axis) in frame.basis.iter_mut().enumerate() {
            *axis = read(matrix, &format!("mat[{index}]"), *axis)?;
        }
    }
    Ok(frame)
}
