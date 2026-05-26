use crate::tuner::TuningParams;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProfileKind {
    Godforge,
    BrokkrsBest,
    DeepCalm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub kind: ProfileKind,
    pub name: String,
    pub tuning: TuningParams,
    pub ram_timings_fastest: bool,
}

pub fn save_profile(_profile: &Profile, _path: &str) -> Result<(), String> {
    Err("Not implemented".into())
}

pub fn load_profile(_path: &str) -> Result<Profile, String> {
    Err("Not implemented".into())
}
