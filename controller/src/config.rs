use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub daemon_addr: String,
    pub daemon_token: Option<String>,
    pub cli_port: u16,
    pub auth_module_path: Option<String>,
    pub chat_module_path: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            daemon_addr: "127.0.0.1:7777".to_string(),
            daemon_token: None,
            cli_port: 8888,
            auth_module_path: None,
            chat_module_path: None,
        }
    }
}

pub fn load() -> Config {
    let config_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("controller.json")))
        .unwrap_or_else(|| "controller.json".into());

    if let Ok(content) = std::fs::read_to_string(&config_path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        Config::default()
    }
}
