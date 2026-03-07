use serde::{Deserialize, Serialize};

fn default_autofix() -> bool {
    false
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PushConfig {
    #[serde(default = "default_autofix")]
    pub autofix: bool,
}

pub fn from_jj(jj: &crate::jj::Jujutsu) -> crate::error::Result<PushConfig> {
    // This fails when the option was never set.
    // Which is ok for us.
    let raw = jj
        .config_get("spr.push")
        .map(|v| format!("spr.push = {}", v))
        .unwrap_or(String::from(""));
    Ok(toml::from_str(raw.as_str())?)
}

impl Default for PushConfig {
    fn default() -> Self {
        toml::from_str("").expect("PushConfig should be defaultable via serde")
    }
}
