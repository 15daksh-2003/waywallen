use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Sequential,
    Shuffle,
    Random,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Shuffle => "shuffle",
            Self::Random => "random",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "sequential" => Some(Self::Sequential),
            "shuffle" => Some(Self::Shuffle),
            "random" => Some(Self::Random),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_roundtrip() {
        for mode in [Mode::Sequential, Mode::Shuffle, Mode::Random] {
            assert_eq!(Mode::from_str(mode.as_str()), Some(mode));
        }
        assert_eq!(Mode::from_str("nonsense"), None);
    }
}
