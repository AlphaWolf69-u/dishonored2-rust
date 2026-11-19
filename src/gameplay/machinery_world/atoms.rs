//! Legacy machinery adapter to the same numerical atom-joint kernel.
use super::MachineryBody;
use crate::{entities::machinery::ConstraintSettings,gameplay::constraint_solver::{joint::BodyInput,atom_stack::AtomStack}};
pub(super) use crate::gameplay::constraint_solver::joint_atoms::{AtomJoint,Transform,inverse_multiply,atom_step};
pub(super) fn cone_stack(settings:&ConstraintSettings,a:&MachineryBody,b:&MachineryBody)->AtomStack {
    crate::gameplay::constraint_solver::joint_atoms::cone_stack(settings,
        BodyInput{pose:&a.motion.pose,inverse_mass:&a.motion.inverse_mass,body_transform:&a.body_transform()},
        BodyInput{pose:&b.motion.pose,inverse_mass:&b.motion.inverse_mass,body_transform:&b.body_transform()})
}
