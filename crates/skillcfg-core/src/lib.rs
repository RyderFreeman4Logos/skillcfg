use std::{
    error::Error as StdError,
    fmt, fs, io,
    path::{Path, PathBuf},
    str::FromStr,
};

#[derive(Debug)]
pub struct Config {
    document: toml::Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigKey {
    raw: String,
    segments: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidKeyError(String);

#[derive(Debug)]
pub enum ConfigError {
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Parse(toml::de::Error),
    AtPath {
        path: PathBuf,
        source: Box<ConfigError>,
    },
    UnsupportedSchema {
        found: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LookupError {
    Missing { key: String },
    NotTable { key: String, at: String },
}

impl Config {
    pub fn parse(source: &str) -> Result<Self, ConfigError> {
        let document: toml::Value = toml::from_str(source).map_err(ConfigError::Parse)?;
        let version = document
            .as_table()
            .and_then(|table| table.get("schema_version"));
        if version.and_then(toml::Value::as_integer) != Some(1) {
            return Err(ConfigError::UnsupportedSchema {
                found: version
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "missing".to_owned()),
            });
        }
        Ok(Self { document })
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let source = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        Self::parse(&source).map_err(|source| ConfigError::AtPath {
            path: path.to_owned(),
            source: Box::new(source),
        })
    }

    pub fn get(&self, key: &ConfigKey) -> Result<&toml::Value, LookupError> {
        let mut value = &self.document;
        for (index, segment) in key.segments.iter().enumerate() {
            let table = value.as_table().ok_or_else(|| LookupError::NotTable {
                key: key.raw.clone(),
                at: key.segments[..index].join("."),
            })?;
            value = table.get(segment).ok_or_else(|| LookupError::Missing {
                key: key.raw.clone(),
            })?;
        }
        Ok(value)
    }
}

impl ConfigKey {
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

impl FromStr for ConfigKey {
    type Err = InvalidKeyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let valid_segment = |segment: &str| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        };
        let segments: Vec<_> = value.split('.').collect();
        if value.is_empty() || segments.iter().any(|segment| !valid_segment(segment)) {
            return Err(InvalidKeyError(value.to_owned()));
        }
        Ok(Self {
            raw: value.to_owned(),
            segments: segments.into_iter().map(str::to_owned).collect(),
        })
    }
}

pub fn render_value(value: &toml::Value) -> String {
    match value {
        toml::Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

impl fmt::Display for InvalidKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid config key {:?}: expected dot-separated ASCII letters, digits, '_' or '-'",
            self.0
        )
    }
}

impl StdError for InvalidKeyError {}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(f, "cannot read config '{}': {source}", path.display())
            }
            Self::Parse(source) => write!(f, "invalid TOML: {source}"),
            Self::AtPath { path, source } => {
                write!(f, "{source} in config '{}'", path.display())
            }
            Self::UnsupportedSchema { found } => {
                write!(f, "unsupported schema_version {found}; expected integer 1")
            }
        }
    }
}

impl StdError for ConfigError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse(source) => Some(source),
            Self::AtPath { source, .. } => Some(source),
            Self::UnsupportedSchema { .. } => None,
        }
    }
}

impl fmt::Display for LookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { key } => write!(f, "config key '{key}' does not exist"),
            Self::NotTable { key, at } => write!(
                f,
                "cannot resolve config key '{key}': '{at}' is not a table"
            ),
        }
    }
}

impl StdError for LookupError {}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigError, ConfigKey, LookupError, render_value};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let base = std::env::temp_dir();
            loop {
                let path = base.join(format!(
                    "skillcfg-core-{}-{}",
                    std::process::id(),
                    NEXT_DIR.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create {}: {error}", path.display()),
                }
            }
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn key(value: &str) -> ConfigKey {
        value.parse().unwrap()
    }

    #[test]
    fn resolves_nested_key_and_toml_value_types() {
        let config = Config::parse(
            "schema_version = 1\nmessage = \"demo\"\nattempts = 7\nratio = 1.5\nenabled = true\nitems = [\"one\", \"two\"]\n[profile]\nkind = \"test\"\n",
        )
        .unwrap();
        assert_eq!(
            config.get(&key("profile.kind")).unwrap().as_str(),
            Some("test")
        );
        assert_eq!(config.get(&key("attempts")).unwrap().as_integer(), Some(7));
        assert_eq!(config.get(&key("ratio")).unwrap().as_float(), Some(1.5));
        assert_eq!(config.get(&key("enabled")).unwrap().as_bool(), Some(true));
        assert_eq!(
            config.get(&key("items")).unwrap().as_array().unwrap().len(),
            2
        );
        assert!(config.get(&key("profile")).unwrap().as_table().is_some());
        assert_eq!(render_value(config.get(&key("message")).unwrap()), "demo");
    }

    #[test]
    fn distinguishes_missing_keys_from_scalar_intermediate_segments() {
        let config = Config::parse("schema_version = 1\nvalue = 3\n").unwrap();
        assert!(matches!(
            config.get(&key("absent")),
            Err(LookupError::Missing { .. })
        ));
        assert!(matches!(
            config.get(&key("value.child")),
            Err(LookupError::NotTable { .. })
        ));
    }

    #[test]
    fn rejects_malformed_toml_and_unsupported_schema_versions() {
        assert!(matches!(
            Config::parse("schema_version = ["),
            Err(ConfigError::Parse(_))
        ));
        assert!(matches!(
            Config::parse("schema_version = 2"),
            Err(ConfigError::UnsupportedSchema { .. })
        ));
        assert!(matches!(
            Config::parse("message = \"no version\""),
            Err(ConfigError::UnsupportedSchema { .. })
        ));
    }

    #[test]
    fn validates_literal_dotted_keys() {
        assert!("model_tiers.review.model_id".parse::<ConfigKey>().is_ok());
        for invalid in ["", ".a", "a.", "a..b", "a b"] {
            assert!(
                invalid.parse::<ConfigKey>().is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn loads_config_through_a_file_symlink() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let actual = temp.path().join("actual.toml");
        let link = temp.path().join("config.toml");
        fs::write(&actual, "schema_version = 1\nmessage = \"linked\"\n").unwrap();
        symlink(&actual, &link).unwrap();
        let config = Config::load(&link).unwrap();
        assert_eq!(
            config.get(&key("message")).unwrap().as_str(),
            Some("linked")
        );
    }
}
