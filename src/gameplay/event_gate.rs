//! 79ebf0/79eb50/79e9c0/79ec50 event-gate handlers.
//! Reads follow connected inputs; writes update the graph instance's own
//! variable storage, which can differ from the connected input's current value.

#[derive(Clone, Copy)]
pub enum GateOutput {
    Open,
    Closed,
}
pub trait GateContext {
    fn is_open(&mut self) -> Result<bool, String>;
    fn auto_close_count(&mut self) -> Result<i32, String>;
    fn set_open(&mut self, value: bool) -> Result<(), String>;
    fn emit(&mut self, output: GateOutput) -> Result<(), String>;
}
#[derive(Default)]
pub struct GateState {
    passed: i32,
}
impl GateState {
    pub fn pass(&mut self, context: &mut impl GateContext) -> Result<(), String> {
        let limit = context.auto_close_count()?;
        if limit >= 0 {
            self.passed = self.passed.wrapping_add(1);
            if limit < self.passed {
                self.close(context)?;
            }
        }
        // The gate closes before this read when the increment exceeds limit.
        let output = if context.is_open()? {
            GateOutput::Open
        } else {
            GateOutput::Closed
        };
        context.emit(output)
    }
    pub fn open(&mut self, context: &mut impl GateContext) -> Result<(), String> {
        context.set_open(true)?;
        self.passed = 0;
        Ok(())
    }
    pub fn close(&mut self, context: &mut impl GateContext) -> Result<(), String> {
        context.set_open(false)
    }
    pub fn toggle(&mut self, context: &mut impl GateContext) -> Result<(), String> {
        if context.is_open()? {
            self.close(context)
        } else {
            self.open(context)
        }
    }
}
