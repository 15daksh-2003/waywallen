use std::path::PathBuf;

pub fn default_config_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("waywallen/config.toml");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config/waywallen/config.toml");
    }
    PathBuf::from("waywallen.toml")
}

pub fn default_db_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("waywallen/waywallen-v2.db");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/share/waywallen/waywallen-v2.db");
    }
    PathBuf::from("waywallen-v2.db")
}

pub fn data_dir() -> PathBuf {
    default_db_path()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn plugin_state_dir() -> PathBuf {
    data_dir().join("plugin-state")
}

pub fn sanitize_path_segment(input: &str) -> String {
    let value: String = input
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if value.is_empty() {
        "default".to_string()
    } else {
        value
    }
}

pub fn remote_content_dir(source_id: &str) -> PathBuf {
    data_dir()
        .join("remote")
        .join(sanitize_path_segment(source_id))
}
