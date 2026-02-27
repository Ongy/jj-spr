use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Icon(String);

impl AsRef<str> for Icon {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

fn default_error() -> Icon {
    Icon(String::from("💔"))
}
fn default_key() -> Icon {
    Icon(String::from("🔑"))
}
fn default_land() -> Icon {
    Icon(String::from("🛬"))
}
fn default_ok() -> Icon {
    Icon(String::from("✅"))
}
fn default_question() -> Icon {
    Icon(String::from("❓"))
}
fn default_info() -> Icon {
    Icon(String::from("❕"))
}
fn default_refresh() -> Icon {
    Icon(String::from("🔁"))
}
fn default_sparkle() -> Icon {
    Icon(String::from("✨"))
}
fn default_stop() -> Icon {
    Icon(String::from("🛑"))
}
fn default_wave() -> Icon {
    Icon(String::from("👋"))
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Icons {
    #[serde(default = "default_error")]
    pub error: Icon,

    #[serde(default = "default_key")]
    pub key: Icon,

    #[serde(default = "default_land")]
    pub land: Icon,

    #[serde(default = "default_ok")]
    pub ok: Icon,

    #[serde(default = "default_question")]
    pub question: Icon,

    #[serde(default = "default_info")]
    pub info: Icon,

    #[serde(default = "default_refresh")]
    pub refresh: Icon,

    #[serde(default = "default_sparkle")]
    pub sparkle: Icon,

    #[serde(default = "default_stop")]
    pub stop: Icon,

    #[serde(default = "default_wave")]
    pub wave: Icon,
}

pub fn from_jj(jj: &crate::jj::Jujutsu) -> crate::error::Result<Icons> {
    // This fails when the option was never set.
    // Which is ok for us.
    let raw = jj.config_get("spr.icons").unwrap_or(String::from("{}"));
    Ok(serde_json::from_str(raw.as_str())?)
}

impl Default for Icons {
    fn default() -> Self {
        serde_json::from_str("{}").expect("Icons should be defaultable via serde")
    }
}

