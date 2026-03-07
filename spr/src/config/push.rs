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
    Ok(PushConfig {
        autofix: jj
            .config_get("spr.push.autofix")
            .map_or(false, |s| s.trim() == "true"),
    })
}

impl Default for PushConfig {
    fn default() -> Self {
        Self { autofix: false }
    }
}
