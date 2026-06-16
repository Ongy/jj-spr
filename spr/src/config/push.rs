use serde::{Deserialize, Serialize};

fn default_autofix() -> bool {
    false
}

fn default_draft() -> bool {
    false
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PushConfig {
    #[serde(default = "default_autofix")]
    pub autofix: bool,
    #[serde(default = "default_draft")]
    pub draft: bool,
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            autofix: false,
            draft: false,
        }
    }
}
