use serde::{de, Deserialize, Deserializer, Serialize};
use std::fmt;
use std::str::FromStr;

fn default_autofix() -> bool {
    false
}

fn default_draft() -> bool {
    false
}

fn default_update_only() -> bool {
    false
}

fn default_modify_comment() -> CommentMode {
    CommentMode::Auto
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Default, clap::ValueEnum)]
#[serde(rename_all = "camelCase")]
pub enum CommentMode {
    #[default]
    Auto,
    On,
    Off,
}

impl FromStr for CommentMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(CommentMode::Auto),
            "on" | "always" | "true" => Ok(CommentMode::On),
            "off" | "never" | "false" => Ok(CommentMode::Off),
            _ => Err(format!("invalid comment mode: {}", s)),
        }
    }
}

impl<'de> Deserialize<'de> for CommentMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CommentModeVisitor;

        impl<'de> de::Visitor<'de> for CommentModeVisitor {
            type Value = CommentMode;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str(
                    "a comment mode (\"auto\", \"on\", \"off\", \"always\", \"never\", \"true\", \"false\", or boolean)",
                )
            }

            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if v {
                    Ok(CommentMode::On)
                } else {
                    Ok(CommentMode::Off)
                }
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                v.parse().map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_any(CommentModeVisitor)
    }
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
    #[serde(default = "default_modify_comment", alias = "modify_comment")]
    pub modify_comment: CommentMode,
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            autofix: false,
            draft: false,
            update_only: false,
            modify_comment: CommentMode::Auto,
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
        assert_eq!(config.modify_comment, CommentMode::Auto);
    }

    #[test]
    fn test_push_config_deserialization_camel_case() {
        let toml_str = r#"
            updateOnly = true
            modifyComment = "on"
        "#;
        let config: PushConfig = toml::from_str(toml_str).unwrap();
        assert!(config.update_only);
        assert_eq!(config.modify_comment, CommentMode::On);
    }

    #[test]
    fn test_push_config_deserialization_snake_case_alias() {
        let toml_str = r#"
            update_only = true
            modify_comment = "off"
        "#;
        let config: PushConfig = toml::from_str(toml_str).unwrap();
        assert!(config.update_only);
        assert_eq!(config.modify_comment, CommentMode::Off);
    }

    #[test]
    fn test_comment_mode_deserialization() {
        let cases = [
            ("auto", CommentMode::Auto),
            ("AUTO", CommentMode::Auto),
            ("on", CommentMode::On),
            ("ON", CommentMode::On),
            ("always", CommentMode::On),
            ("true", CommentMode::On),
            ("off", CommentMode::Off),
            ("OFF", CommentMode::Off),
            ("never", CommentMode::Off),
            ("false", CommentMode::Off),
        ];

        for (input, expected) in cases {
            let toml_str = format!("modifyComment = \"{}\"", input);
            let config: PushConfig = toml::from_str(&toml_str).unwrap();
            assert_eq!(config.modify_comment, expected, "Failed for input: {}", input);
        }

        // Test boolean values in TOML
        let config_bool_true: PushConfig = toml::from_str("modifyComment = true").unwrap();
        assert_eq!(config_bool_true.modify_comment, CommentMode::On);

        let config_bool_false: PushConfig = toml::from_str("modifyComment = false").unwrap();
        assert_eq!(config_bool_false.modify_comment, CommentMode::Off);
    }
}


