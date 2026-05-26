pub struct SensorReading {
    pub label: String,
    pub value: f64,
    pub unit: String,
}

pub struct Monitor {
    running: bool,
}

impl Monitor {
    pub fn new() -> Self {
        Self { running: false }
    }

    pub fn start(&mut self) -> Result<(), String> {
        Err("Not implemented".into())
    }

    pub fn stop(&mut self) {}

    pub fn read_sensors(&self) -> Vec<SensorReading> {
        vec![]
    }

    pub fn check_whea_errors(&self) -> Vec<String> {
        vec![]
    }
}
