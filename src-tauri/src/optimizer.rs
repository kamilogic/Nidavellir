use crate::profile::Profile;

pub enum OptimizerState {
    Idle,
    Sweeping,
    Modeling,
    Done { profiles: [Profile; 3] },
}

pub struct Optimizer {
    state: OptimizerState,
}

impl Optimizer {
    pub fn new() -> Self {
        Self {
            state: OptimizerState::Idle,
        }
    }

    pub fn start_sweep(&mut self) -> Result<(), String> {
        Err("Not implemented".into())
    }

    pub fn state(&self) -> &OptimizerState {
        &self.state
    }

    pub fn abort(&mut self) {
        self.state = OptimizerState::Idle;
    }
}
