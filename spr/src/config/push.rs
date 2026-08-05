use serde::{Deserialize, Serialize};

fn default_autofix() -> bool {
    false
}

fn default_draft() -> bool {
    false
}

fn default_update_only() -> bool {
    false
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PushConfig {
    #[serde(default = "default_autofix")]
    pub autofix: bool,
    #[serde(default = "default_draft")]
    pub draft: bool,
    #[serde(default = "default_update_only", alias = "update_only")]
    pub update_only: bool,
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            autofix: false,
            draft: false,
            update_only: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_config_default() {
        let config = PushConfig::default();
        assert!(!config.autofix);
        assert!(!config.draft);
        assert!(!config.update_only);
    }

    #[test]
    fn test_push_config_deserialization_camel_case() {
        let toml_str = r#"
            updateOnly = true
        "#;
        let config: PushConfig = toml::from_str(toml_str).unwrap();
        assert!(config.update_only);
    }

    #[test]
    fn test_push_config_deserialization_snake_case_alias() {
        let toml_str = r#"
            update_only = true
        "#;
        let config: PushConfig = toml::from_str(toml_str).unwrap();
        assert!(config.update_only);
    }
}

