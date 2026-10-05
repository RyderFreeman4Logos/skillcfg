//! Optional skill-local bindings; values always remain in the shared config.
use crate::{Config, ConfigKey, render_value};
use std::{collections::BTreeMap, fs, path::Path};

/// One validated alias binding and its manifest source line.
#[derive(Debug)]
pub struct Binding {
    /// Stable canonical dotted key.
    pub key: ConfigKey,
    /// One-based assignment line in the manifest.
    pub line: usize,
}
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct VisibleSource {
    visible: Option<BTreeMap<String, toml::Spanned<toml::Value>>>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct OpaqueSource {
    opaque: Option<BTreeMap<String, toml::Spanned<toml::Value>>>,
}

/// Optional manifest. Alias maps are sorted; opaque bindings are never selected implicitly.
#[derive(Debug, Default)]
pub struct Manifest {
    /// Values required for agent reasoning.
    pub visible: BTreeMap<String, Binding>,
    /// Values consumed only by skill-owned scripts.
    pub opaque: BTreeMap<String, Binding>,
}
impl Manifest {
    /// Load a regular UTF-8 file (symlinks supported). A genuinely absent optional file is
    /// empty; a broken symlink is an error. Diagnostics contain no source excerpts/values.
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("cannot read manifest {:?}: {e}", path)),
            Ok(_) => {}
        }
        if !fs::metadata(path).is_ok_and(|m| m.is_file()) {
            return Err(format!(
                "manifest is not a readable regular file: {:?}",
                path
            ));
        }
        let source = fs::read_to_string(path)
            .map_err(|e| format!("cannot read manifest {:?}: {e}", path))?;
        Self::parse(&source).map_err(|e| format!("{e} in manifest {:?}", path))
    }
    /// Parse schema v1 with string bindings. Duplicate TOML aliases fail parsing; aliases
    /// shared across visible/opaque fail admission. No values are included in errors.
    pub fn parse(source: &str) -> Result<Self, String> {
        let config = Config::parse(source).map_err(|e| e.to_string())?;
        let visible_locations = toml::from_str::<VisibleSource>(source).ok();
        let opaque_locations = toml::from_str::<OpaqueSource>(source).ok();
        let mut manifest = Self::default();
        for (class, bindings) in [
            ("visible", &mut manifest.visible),
            ("opaque", &mut manifest.opaque),
        ] {
            if let Some(value) = config.document.get(class) {
                let table = value
                    .as_table()
                    .ok_or_else(|| format!("manifest {class} must be a table"))?;
                for (alias, value) in table {
                    if alias.is_empty()
                        || !alias
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    {
                        return Err(format!(
                            "invalid manifest alias in {class}; expected ASCII letters, digits, _ or -"
                        ));
                    }
                    let key = value.as_str().ok_or_else(|| {
                        format!("manifest {class}.{alias} must bind a string key")
                    })?;
                    let key = key.parse::<ConfigKey>().map_err(|_| {
                        format!("invalid canonical key in manifest {class}.{alias}")
                    })?;
                    let span = match class {
                        "visible" => visible_locations
                            .as_ref()
                            .and_then(|locations| locations.visible.as_ref())
                            .and_then(|aliases| aliases.get(alias)),
                        "opaque" => opaque_locations
                            .as_ref()
                            .and_then(|locations| locations.opaque.as_ref())
                            .and_then(|aliases| aliases.get(alias)),
                        _ => None,
                    };
                    let location = span
                        .and_then(|value| source.get(..value.span().start))
                        .map(|prefix| prefix.bytes().filter(|byte| *byte == b'\n').count() + 1)
                        .ok_or_else(|| format!("cannot locate manifest {class}.{alias} source"))?;
                    bindings.insert(
                        alias.clone(),
                        Binding {
                            key,
                            line: location,
                        },
                    );
                }
            }
        }
        if let Some(alias) = manifest
            .visible
            .keys()
            .find(|a| manifest.opaque.contains_key(*a))
        {
            return Err(format!(
                "duplicate manifest alias {alias:?} across visible and opaque"
            ));
        }
        Ok(manifest)
    }
}
/// Compact JSON string escaping, also used to quote ambiguous KV strings.
pub fn quote(value: &str) -> String {
    serde_json::Value::String(value.to_owned()).to_string()
}
/// Serialize already-resolved pairs atomically. KV preserves input order, escapes complex
/// strings as JSON literals, and uses compact JSON for arrays/tables. JSON preserves pair
/// order in the emitted object. Duplicate labels fail, rather than becoming ambiguous JSON.
pub fn render_pairs(pairs: &[(&str, &toml::Value)], json: bool) -> Result<String, String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut output = Vec::new();
    for (label, value) in pairs {
        if !seen.insert(*label) {
            return Err(format!("duplicate output key {label:?}"));
        }
        let encoded = json_value(value)?;
        if json {
            output.push(format!("{}:{}", quote(label), encoded));
        } else {
            let rendered = match value {
                toml::Value::String(s)
                    if !s.is_empty()
                        && s.trim() == s
                        && !s.chars().any(char::is_control)
                        && !s.starts_with(['"', '[', '{'])
                        && !matches!(s.as_str(), "true" | "false" | "null")
                        && s.parse::<f64>().is_err() =>
                {
                    s.clone()
                }
                toml::Value::String(s) => quote(s),
                toml::Value::Array(_) | toml::Value::Table(_) => encoded.to_string(),
                _ => render_value(value),
            };
            output.push(format!("{label}={rendered}"));
        }
    }
    if json {
        Ok(format!("{{{}}}\n", output.join(",")))
    } else if output.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("{}\n", output.join("\n")))
    }
}
fn json_value(value: &toml::Value) -> Result<serde_json::Value, String> {
    match value {
        toml::Value::Datetime(value) => Ok(serde_json::Value::String(value.to_string())),
        toml::Value::Float(f) if !f.is_finite() => {
            Err("non-finite floats cannot be represented in batch JSON/KV".to_owned())
        }
        toml::Value::Array(items) => items
            .iter()
            .map(json_value)
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        toml::Value::Table(items) => items
            .iter()
            .map(|(k, v)| Ok((k.clone(), json_value(v)?)))
            .collect::<Result<serde_json::Map<_, _>, String>>()
            .map(serde_json::Value::Object),
        _ => serde_json::to_value(value)
            .map_err(|_| "value cannot be represented in JSON".to_owned()),
    }
}
