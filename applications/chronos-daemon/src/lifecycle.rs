//! One serialized request obligation. Metrics never drive this state machine.
use chronos_wire::control::LifecycleV1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleError {
    NotRunning,
    Busy,
    NoObligation,
    NotDrained,
}
pub struct Lifecycle {
    state: LifecycleV1,
    active: u8,
}
impl Default for Lifecycle {
    fn default() -> Self {
        Self {
            state: LifecycleV1::Running,
            active: 0,
        }
    }
}
impl Lifecycle {
    pub fn state(&self) -> LifecycleV1 {
        self.state
    }
    pub fn active(&self) -> u8 {
        self.active
    }
    pub fn ready(&self) -> bool {
        self.state == LifecycleV1::Running && self.active == 0
    }
    pub fn admit(&mut self) -> Result<(), LifecycleError> {
        if self.state != LifecycleV1::Running {
            return Err(LifecycleError::NotRunning);
        }
        if self.active != 0 {
            return Err(LifecycleError::Busy);
        }
        self.active = 1;
        Ok(())
    }
    pub fn finish(&mut self) -> Result<(), LifecycleError> {
        if self.active != 1 {
            return Err(LifecycleError::NoObligation);
        }
        self.active = 0;
        Ok(())
    }
    pub fn drain(&mut self) -> Result<(), LifecycleError> {
        match self.state {
            LifecycleV1::Running => self.state = LifecycleV1::Draining,
            LifecycleV1::Draining | LifecycleV1::Stopped => {}
            LifecycleV1::Fenced => return Err(LifecycleError::NotRunning),
        }
        Ok(())
    }
    pub fn stop(&mut self) -> Result<(), LifecycleError> {
        if self.active != 0 || !matches!(self.state, LifecycleV1::Draining | LifecycleV1::Stopped) {
            return Err(LifecycleError::NotDrained);
        }
        self.state = LifecycleV1::Stopped;
        Ok(())
    }
    pub fn fence(&mut self) {
        self.state = LifecycleV1::Fenced;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drain_rejects_new_work_and_waits_for_its_one_obligation() {
        let mut state = Lifecycle::default();
        assert!(state.ready());
        state.admit().unwrap();
        assert_eq!(state.admit(), Err(LifecycleError::Busy));
        state.drain().unwrap();
        assert!(!state.ready());
        assert_eq!(state.admit(), Err(LifecycleError::NotRunning));
        assert_eq!(state.stop(), Err(LifecycleError::NotDrained));
        state.finish().unwrap();
        state.stop().unwrap();
        state.stop().unwrap();
        assert_eq!(state.state(), LifecycleV1::Stopped);
    }
    #[test]
    fn all_five_step_programs_preserve_single_obligation_and_terminal_fence() {
        for program in 0..3125_u32 {
            let mut code = program;
            let mut state = Lifecycle::default();
            let mut terminal_fence = false;
            for _ in 0..5 {
                let before = (state.state(), state.active());
                let result = match code % 5 {
                    0 => state.admit(),
                    1 => state.finish(),
                    2 => state.drain(),
                    3 => state.stop(),
                    4 => {
                        state.fence();
                        terminal_fence = true;
                        Ok(())
                    }
                    _ => unreachable!(),
                };
                code /= 5;
                if result.is_err() {
                    assert_eq!((state.state(), state.active()), before);
                }
                assert!(state.active() <= 1);
                if terminal_fence {
                    assert_eq!(state.state(), LifecycleV1::Fenced);
                }
                if state.state() == LifecycleV1::Stopped {
                    assert_eq!(state.active(), 0);
                }
            }
        }
    }
}
