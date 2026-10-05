/// Canonical skill discovery and configurable roots.
pub mod discovery;

use std::{
    error::Error as StdError,
    fmt, fs, io,
    path::{Path, PathBuf},
    str::FromStr,
};

/// A parsed TOML document with integer `schema_version = 1`.
#[derive(Debug)]
pub struct Config {
    document: toml::Value,
}

/// A nonempty dotted path of ASCII letters, digits, `_` and `-`.
/// Segments cannot be empty, quoted, or Unicode; dots always separate tables.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigKey {
    raw: String,
    segments: Vec<String>,
}

/// A key outside the literal ASCII dotted-path grammar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidKeyError(String);

/// Read, syntax, or schema failure; diagnostics never include TOML source values.
#[derive(Debug)]
pub enum ConfigError {
    /// The file could not be read as UTF-8.
    Read {
        /// Requested file path (symlinks are followed).
        path: PathBuf,
        /// Underlying filesystem or UTF-8 error.
        source: io::Error,
    },
    /// Invalid TOML with an optional one-based line and column.
    Parse(Option<(usize, usize)>),
    /// Adds the requested path to a syntax or schema error.
    AtPath {
        /// Requested file path.
        path: PathBuf,
        /// Sanitized syntax or schema error.
        source: Box<ConfigError>,
    },
    /// The schema version is absent, not an integer, or not version 1.
    UnsupportedSchema {
        /// Only the TOML type name, or `missing`; never the actual value.
        found: String,
    },
}

/// Lookup failure without disclosing any stored values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LookupError {
    /// A path segment is absent.
    Missing {
        /// Full requested key.
        key: String,
    },
    /// An intermediate path resolves to a non-table value.
    NotTable {
        /// Full requested key.
        key: String,
        /// Prefix that resolved to a non-table value.
        at: String,
    },
}

impl Config {
    /// Parse UTF-8 TOML and require integer `schema_version = 1`.
    /// Syntax and schema errors contain locations/types, not source values.
    pub fn parse(source: &str) -> Result<Self, ConfigError> {
        let document: toml::Value = toml::from_str(source).map_err(|error: toml::de::Error| {
            let location = error.span().map(|span| {
                let prefix = &source[..span.start];
                let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
                let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
                (line, column)
            });
            ConfigError::Parse(location)
        })?;
        let version = document
            .as_table()
            .and_then(|table| table.get("schema_version"));
        if version.and_then(toml::Value::as_integer) != Some(1) {
            return Err(ConfigError::UnsupportedSchema {
                found: version
                    .map(|value| value.type_str().to_owned())
                    .unwrap_or_else(|| "missing".to_owned()),
            });
        }
        Ok(Self { document })
    }

    /// Read a UTF-8 file (following symlinks), then parse it; errors retain the path.
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

    /// Borrow the exact value; missing segments and non-table parents are distinct errors.
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
    /// Return the validated, unchanged dotted path.
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

/// Render strings verbatim (including existing newlines), other values as TOML text.
/// Arrays/tables use inline TOML. No output newline is added; the CLI owns that policy.
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
            Self::Parse(Some((line, column))) => {
                write!(f, "invalid TOML at line {line}, column {column}")
            }
            Self::Parse(None) => write!(f, "invalid TOML"),
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
            Self::Parse(_) => None,
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
