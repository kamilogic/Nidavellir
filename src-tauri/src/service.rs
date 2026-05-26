pub struct Service {
    enabled: bool,
}

impl Service {
    pub fn new() -> Self {
        Self { enabled: false }
    }

    pub fn install_auto_apply(&mut self) -> Result<(), String> {
        Err("Not implemented".into())
    }

    pub fn uninstall_auto_apply(&mut self) -> Result<(), String> {
        Err("Not implemented".into())
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}
