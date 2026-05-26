pub enum TestTarget {
    Cpu,
    Gpu,
    Ram,
    All,
}

pub enum TestResult {
    Stable,
    Unstable { details: String },
    Crash { bugcheck: Option<u32> },
}

pub fn run_stress_test(_target: TestTarget, _duration_secs: u32) -> TestResult {
    TestResult::Unstable {
        details: "Not implemented".into(),
    }
}
