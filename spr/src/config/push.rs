use serde::{Deserialize, Serialize};

fn default_autofix() -> bool {
    false
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PushConfig {
    #[serde(default = "default_autofix")]
    pub autofix: bool,
}

impl Default for PushConfig {
    fn default() -> Self {
        Self { autofix: false }
    }
}
