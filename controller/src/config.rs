use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub struct Config {
    pub daemon_addr: String,
    pub daemon_token: Option<String>,
    pub cli_port: u16,
    pub auth_module_path: Option<String>,
    pub chat_module_path: Option<String>,
    #[serde(default)]
    pub auth_config: Value,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            daemon_addr: "127.0.0.1:7777".to_string(),
            daemon_token: None,
            cli_port: 8888,
            auth_module_path: None,
            chat_module_path: None,
            auth_config: Value::Null,
        }
    }
}

pub fn load() -> Config {
    let candidates = [
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("controller.json"))),
        Some("controller.json".into()),
    ];

    for path in candidates.iter().flatten() {
        if let Ok(content) = std::fs::read_to_string(path) {
            return serde_json::from_str(&content).unwrap_or_default();
        }
    }

    Config::default()
}
