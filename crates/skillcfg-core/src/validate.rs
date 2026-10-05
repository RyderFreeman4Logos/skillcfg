//! Static dependency checking only: scripts are never executed and expressions are never expanded.
use crate::{
    Config, ConfigKey,
    discovery::{Diagnostic, Skill},
    display_path,
    manifest::Manifest,
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// A declared/static key, or an explicitly unverifiable invocation.
#[derive(Debug)]
pub struct Reference {
    /// None means the script expression cannot be resolved statically.
    pub key: Option<ConfigKey>,
    /// Source manifest or script, preserving its skill-local path.
    pub path: PathBuf,
    /// One-based source line.
    pub line: usize,
    /// `visible.ALIAS`, `opaque.ALIAS`, or `script`.
    pub origin: String,
}
/// The same read-only analysis powers validation and impact inspection.
#[derive(Debug, Default)]
pub struct Analysis {
    /// Sorted by path, line, origin; dynamic expressions have no claimed key.
    pub references: Vec<Reference>,
    /// Errors/warnings without resolved config values or script excerpts.
    pub diagnostics: Vec<Diagnostic>,
}
/// Parse optional dependencies and recursively scan regular UTF-8 files under scripts/.
/// Follow script links, deduplicate file identities, stop directory cycles, and keep errors
/// independent so one malformed manifest does not suppress script findings.
pub fn analyze(skill: &Skill, config: &Config) -> Analysis {
    let mut result = Analysis::default();
    let manifest_path = skill.canonical_dir.join("skillcfg.toml");
    let manifest = match Manifest::load(&manifest_path) {
        Ok(manifest) => manifest,
        Err(message) => {
            result.diagnostics.push(Diagnostic {
                error: true,
                message,
            });
            Manifest::default()
        }
    };
    for (class, bindings) in [("visible", &manifest.visible), ("opaque", &manifest.opaque)] {
        for (alias, binding) in bindings {
            result.references.push(Reference {
                key: Some(binding.key.clone()),
                path: manifest_path.clone(),
                line: binding.line,
                origin: format!("{class}.{alias}"),
            });
        }
    }
    let scripts = skill.canonical_dir.join("scripts");
    if fs::symlink_metadata(&scripts).is_ok() {
        scan_files(&scripts, &mut BTreeSet::new(), &mut result);
    }
    for reference in &result.references {
        let location = format!("{}:{}", display_path(&reference.path), reference.line);
        if let Some(key) = &reference.key {
            if let Err(error) = config.get(key) {
                result.diagnostics.push(Diagnostic {
                    error: true,
                    message: format!("{location}: {error}"),
                });
            }
            if reference.origin == "script"
                && !manifest
                    .visible
                    .values()
                    .chain(manifest.opaque.values())
                    .any(|b| &b.key == key)
            {
                result.diagnostics.push(Diagnostic {
                    error: false,
                    message: format!("{location}: undeclared script key {}", key.as_str()),
                });
            }
        } else {
            result.diagnostics.push(Diagnostic {
                error: false,
                message: format!(
                    "{location}: unverifiable skillcfg invocation (dynamic or unsupported syntax)"
                ),
            });
        }
    }
    for (alias, binding) in &manifest.opaque {
        if !result
            .references
            .iter()
            .any(|r| r.origin == "script" && r.key.as_ref() == Some(&binding.key))
        {
            result.diagnostics.push(Diagnostic {
                error: false,
                message: format!(
                    "{}:{}: opaque dependency {alias:?} not statically used",
                    display_path(&manifest_path),
                    binding.line
                ),
            });
        }
    }
    result
        .references
        .sort_by(|a, b| (&a.path, a.line, &a.origin).cmp(&(&b.path, b.line, &b.origin)));
    result.diagnostics.sort_by(|a, b| a.message.cmp(&b.message));
    result
}
/// Build an invocation-local reverse index from analyzed consumers. Unverifiable script
/// expressions are excluded, never attributed to any key. Input order controls reference order.
pub fn reverse_index(
    analyses: &[(String, Analysis)],
) -> std::collections::BTreeMap<&str, Vec<(&str, &Reference)>> {
    let mut index = std::collections::BTreeMap::<&str, Vec<(&str, &Reference)>>::new();
    for (skill, analysis) in analyses {
        for reference in &analysis.references {
            if let Some(key) = &reference.key {
                index
                    .entry(key.as_str())
                    .or_default()
                    .push((skill.as_str(), reference));
            }
        }
    }
    index
}

fn scan_files(path: &Path, visited: &mut BTreeSet<PathBuf>, result: &mut Analysis) {
    let canonical = match fs::canonicalize(path) {
        Ok(p) => p,
        Err(e) => {
            result.diagnostics.push(Diagnostic {
                error: true,
                message: format!("cannot scan {:?}: {e}", path),
            });
            return;
        }
    };
    if !visited.insert(canonical) {
        return;
    }
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            result.diagnostics.push(Diagnostic {
                error: true,
                message: format!("cannot inspect {:?}: {e}", path),
            });
            return;
        }
    };
    if metadata.is_dir() {
        if visited.len() > 10000 {
            result.diagnostics.push(Diagnostic {
                error: true,
                message: format!("script traversal limit at {:?}", path),
            });
            return;
        }
        match fs::read_dir(path).and_then(|entries| entries.collect::<Result<Vec<_>, _>>()) {
            Ok(mut entries) => {
                entries.sort_by_key(|e| e.file_name());
                for entry in entries {
                    scan_files(&entry.path(), visited, result);
                }
            }
            Err(e) => result.diagnostics.push(Diagnostic {
                error: true,
                message: format!("cannot list scripts {:?}: {e}", path),
            }),
        }
    } else if metadata.is_file() {
        match fs::read_to_string(path) {
            Ok(source) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if source.starts_with("#!") && metadata.permissions().mode() & 0o111 == 0 {
                        result.diagnostics.push(Diagnostic {
                            error: false,
                            message: format!("script is not executable: {:?}", path),
                        });
                    }
                }
                for (line, key) in scan_source(&source) {
                    result.references.push(Reference {
                        key,
                        path: path.to_owned(),
                        line,
                        origin: "script".to_owned(),
                    });
                }
            }
            Err(e) => result.diagnostics.push(Diagnostic {
                error: true,
                message: format!("cannot read script {:?}: {e}", path),
            }),
        }
    } else {
        result.diagnostics.push(Diagnostic {
            error: true,
            message: format!("script is not a regular file: {:?}", path),
        });
    }
}
mod scanner;

/// Recognize simple shell-position `skillcfg get KEY` and active command substitutions.
/// Lexical context spans physical lines; data/comments/heredoc bodies never claim keys.
/// Unsupported invocation syntax remains unknown, without expansion or execution.
pub fn scan_source(source: &str) -> Vec<(usize, Option<ConfigKey>)> {
    scanner::scan(source)
}
