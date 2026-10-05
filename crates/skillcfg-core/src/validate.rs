//! Static dependency checking only: scripts are never executed and expressions are never expanded.
use crate::{
    Config, ConfigKey,
    discovery::{Diagnostic, Skill},
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
        let location = format!("{}:{}", reference.path.display(), reference.line);
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
                    manifest_path.display(),
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
/// Recognize simple shell-position `skillcfg get KEY` and command substitutions. Quoted
/// literal keys are accepted; comments/ordinary strings are skipped. Dollar expressions,
/// backslashes, unsupported commands/options, and missing keys remain unknown.
/// ponytail: line-local lexer, not a Bash AST; multiline/complex shell programs need their own tests.
pub fn scan_source(source: &str) -> Vec<(usize, Option<ConfigKey>)> {
    let mut references = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let bytes = line.as_bytes();
        let (mut at, mut quote, mut start) = (0, None, true);
        while at < bytes.len() {
            let b = bytes[at];
            if quote != Some(b'\'') && bytes[at..].starts_with(b"$(") {
                if let Some(key) = invocation(&line[at + 2..]) {
                    references.push((index + 1, key));
                }
                at += 2;
                start = false;
                continue;
            }
            if let Some(q) = quote {
                if b == q {
                    quote = None;
                } else if b == b'\\' && q == b'"' {
                    at += 1;
                }
                at += 1;
                continue;
            }
            if b == b'#' && (at == 0 || bytes[at - 1].is_ascii_whitespace()) {
                break;
            }
            if b == b'\'' || b == b'"' {
                quote = Some(b);
                start = false;
            } else if b == b'\\' {
                at += 1;
                start = false;
            } else if b";|&(".contains(&b) {
                start = true;
            } else if !b.is_ascii_whitespace() {
                if start {
                    if let Some(key) = invocation(&line[at..]) {
                        references.push((index + 1, key));
                    }
                }
                start = false;
            }
            at += 1;
        }
    }
    references
}
fn invocation(source: &str) -> Option<Option<ConfigKey>> {
    let source = source.trim_start();
    let rest = source.strip_prefix("skillcfg")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let (command, rest, command_dynamic) = word(rest)?;
    if command != "get" || command_dynamic {
        return Some(None);
    }
    let Some((key, rest, dynamic)) = word(rest) else {
        return Some(None);
    };
    if dynamic
        || rest
            .trim_start()
            .starts_with(|ch: char| !matches!(ch, ')' | ';' | '|' | '&' | '#' | '>'))
    {
        return Some(None);
    }
    Some(key.parse().ok())
}
fn word(source: &str) -> Option<(String, &str, bool)> {
    let source = source.trim_start();
    if source.is_empty() || source.starts_with([')', ';', '|', '&', '#', '>']) {
        return None;
    }
    let mut text = String::new();
    let mut quote = None;
    let mut dynamic = false;
    let mut end = source.len();
    for (index, ch) in source.char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                if q == '"' && matches!(ch, '$' | '`' | '\\') {
                    dynamic = true;
                }
                text.push(ch);
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch.is_whitespace() || matches!(ch, ')' | ';' | '|' | '&' | '>') {
            end = index;
            break;
        } else {
            if matches!(ch, '$' | '`' | '\\' | '(') {
                dynamic = true;
            }
            text.push(ch);
        }
    }
    Some((text, &source[end..], dynamic || quote.is_some()))
}
