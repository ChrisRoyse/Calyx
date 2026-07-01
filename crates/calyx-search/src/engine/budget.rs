use crate::error::CliResult;

pub struct SearchBudget<'a> {
    check: Option<&'a mut dyn FnMut(&'static str, usize) -> CliResult>,
}

impl<'a> SearchBudget<'a> {
    pub fn disabled() -> Self {
        Self { check: None }
    }

    pub fn new(check: &'a mut dyn FnMut(&'static str, usize) -> CliResult) -> Self {
        Self { check: Some(check) }
    }

    pub(super) fn check(&mut self, phase: &'static str, processed: usize) -> CliResult {
        if let Some(check) = self.check.as_mut() {
            check(phase, processed)?;
        }
        Ok(())
    }
}
