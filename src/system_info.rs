use std::fs;
#[cfg(test)]
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct SystemInfo {
    os_name: String,
}

impl SystemInfo {
    pub fn load() -> Self {
        match fs::read_to_string("/etc/os-release") {
            Ok(content) => Self {
                os_name: parse_os_name(&content).unwrap_or_default(),
            },
            Err(error) => {
                log::warn!("read /etc/os-release: {error}");
                Self::default()
            }
        }
    }

    pub fn os_name(&self) -> &str {
        &self.os_name
    }

    #[cfg(test)]
    fn load_from(path: &Path) -> std::io::Result<Self> {
        let content = fs::read_to_string(path)?;
        Ok(Self {
            os_name: parse_os_name(&content).unwrap_or_default(),
        })
    }
}

fn parse_os_name(content: &str) -> Option<String> {
    content.lines().filter_map(parse_name_line).last()
}

fn parse_name_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    if key.trim() != "NAME" {
        return None;
    }
    parse_value(value.trim())
}

fn parse_value(value: &str) -> Option<String> {
    if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return unescape(inner);
    }
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        return Some(inner.to_owned());
    }
    unescape(value)
}

fn unescape(value: &str) -> Option<String> {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            result.push(chars.next()?);
        } else {
            result.push(ch);
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_os_release_name() {
        assert_eq!(
            parse_os_name("ID=fedora\nNAME=\"Fedora Linux\"\n"),
            Some("Fedora Linux".to_owned())
        );
        assert_eq!(parse_os_name("NAME=Ubuntu\n"), Some("Ubuntu".to_owned()));
        assert_eq!(
            parse_os_name("NAME='Arch Linux'\n"),
            Some("Arch Linux".to_owned())
        );
        assert_eq!(
            parse_os_name("NAME=First\nNAME=Second\\ Linux\n"),
            Some("Second Linux".to_owned())
        );
    }

    #[test]
    fn loads_os_release_from_file() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "NAME=\"Test Linux\"").unwrap();

        let info = SystemInfo::load_from(file.path()).unwrap();

        assert_eq!(info.os_name, "Test Linux");
    }
}
