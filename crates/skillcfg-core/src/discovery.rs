//! Read-only discovery: directory identity is canonical, exposures remain lexical.
use crate::Config;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// One canonical skill, with every encountered installation path.
#[derive(Debug)]
pub struct Skill {
    /// Frontmatter `name`, or canonical directory basename.
    pub name: String,
    /// Resolved skill directory (not the SKILL.md link target).
    pub canonical_dir: PathBuf,
    /// Sorted, deduplicated lexical paths exposing this directory.
    pub exposures: Vec<PathBuf>,
}
/// Discovery problem; errors prevent unambiguous selection, warnings allow enumeration.
#[derive(Debug)]
pub struct Diagnostic {
    /// Whether ordinary discovery must fail (strict validation promotes warnings).
    pub error: bool,
    /// Content-free actionable location and category.
    pub message: String,
}
/// Deterministically ordered skills and diagnostics, including partial traversal results.
#[derive(Debug, Default)]
pub struct Discovery {
    /// Sorted by logical name, then canonical path.
    pub skills: Vec<Skill>,
    /// Sorted by diagnostic text.
    pub diagnostics: Vec<Diagnostic>,
}
/// Return configured roots/ignore patterns. Relative roots use the config's lexical parent;
/// `~` uses the supplied home. Absent roots use only four conventional skill directories.
pub fn settings(
    config: Option<&Config>,
    config_path: &Path,
    home: &Path,
    roots_overridden: bool,
) -> Result<(Vec<PathBuf>, Vec<String>, bool), String> {
    let mut roots = Vec::new();
    let mut ignores = vec![".git", "target", "node_modules", ".cache", "__pycache__"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut configured = false;
    if let Some(config) = config {
        let discovery = config.document.get("discovery");
        if let Some(value) = discovery {
            let table = value.as_table().ok_or("discovery must be a table")?;
            for field in ["roots", "ignore"] {
                if let Some(value) = table.get(field) {
                    let values = value
                        .as_array()
                        .ok_or_else(|| format!("discovery.{field} must be an array of strings"))?;
                    if field == "roots" {
                        configured = true;
                    }
                    for value in values {
                        let value = value.as_str().filter(|s| !s.is_empty()).ok_or_else(|| {
                            format!("discovery.{field} must contain nonempty strings")
                        })?;
                        if field == "roots" {
                            if roots_overridden {
                                continue;
                            }
                            let path = expand_path(value, home)?;
                            roots.push(if path.is_absolute() {
                                path
                            } else {
                                config_path.parent().unwrap_or(Path::new(".")).join(path)
                            });
                        } else {
                            ignores.push(value.to_owned());
                        }
                    }
                }
            }
        }
    }
    if !configured && !roots_overridden {
        if home.as_os_str().is_empty() {
            return Err("HOME is required for conventional discovery roots".to_owned());
        }
        roots = [".codex", ".hermes", ".claude", ".agents"]
            .map(|agent| home.join(agent).join("skills"))
            .to_vec();
    }
    Ok((roots, ignores, configured))
}
/// Expand only `~` and `~/`; other tilde forms fail rather than naming the wrong user.
pub fn expand_path(value: &str, home: &Path) -> Result<PathBuf, String> {
    if (value == "~" || value.starts_with("~/")) && home.as_os_str().is_empty() {
        return Err("HOME is required for ~ expansion".to_owned());
    }
    if value == "~" {
        Ok(home.to_owned())
    } else if let Some(rest) = value.strip_prefix("~/") {
        Ok(home.join(rest))
    } else if value.starts_with('~') {
        Err("only ~ and ~/ home expansion are supported".to_owned())
    } else {
        Ok(PathBuf::from(value))
    }
}
/// Traverse roots following symlinks, including external targets. Missing conventional roots
/// are silent; explicit missing roots are errors. Active ancestors stop cycles, while aliases
/// are traversed to retain child exposures. Canonical SKILL.md names are read once.
pub fn discover(roots: &[PathBuf], ignores: &[String], explicit: bool) -> Discovery {
    discover_inner(roots, ignores, explicit, false)
}
/// Discover one selected Skill without admitting nested Skills as command dependencies.
pub fn discover_selected(root: &Path) -> Discovery {
    discover_inner(&[root.to_owned()], &[], true, true)
}
fn discover_inner(
    roots: &[PathBuf],
    ignores: &[String],
    explicit: bool,
    selected_only: bool,
) -> Discovery {
    let mut skills = BTreeMap::<PathBuf, Skill>::new();
    let mut diagnostics = Vec::new();
    for root in roots {
        if explicit && fs::metadata(root).is_ok_and(|m| !m.is_dir()) {
            diagnostics.push(Diagnostic {
                error: true,
                message: format!("discovery root must be a directory: {:?}", root),
            });
            continue;
        }
        if !explicit
            && fs::symlink_metadata(root).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            continue;
        }
        walk(
            root,
            ignores,
            &mut BTreeSet::new(),
            &mut skills,
            &mut diagnostics,
            selected_only,
        );
    }
    let mut skills: Vec<_> = skills.into_values().collect();
    skills.sort_by(|a, b| (&a.name, &a.canonical_dir).cmp(&(&b.name, &b.canonical_dir)));
    for skill in &mut skills {
        skill.exposures.sort();
        skill.exposures.dedup();
    }
    for pair in skills.windows(2) {
        if pair[0].name == pair[1].name {
            diagnostics.push(Diagnostic {
                error: true,
                message: format!(
                    "name collision {:?}: {:?} exposures {:?}; {:?} exposures {:?}",
                    pair[0].name,
                    pair[0].canonical_dir,
                    pair[0].exposures,
                    pair[1].canonical_dir,
                    pair[1].exposures
                ),
            });
        }
    }
    diagnostics.sort_by(|a, b| a.message.cmp(&b.message));
    diagnostics.dedup_by(|a, b| a.error == b.error && a.message == b.message);
    Discovery {
        skills,
        diagnostics,
    }
}
fn walk(
    path: &Path,
    ignores: &[String],
    active: &mut BTreeSet<PathBuf>,
    skills: &mut BTreeMap<PathBuf, Skill>,
    diagnostics: &mut Vec<Diagnostic>,
    selected_only: bool,
) {
    let canonical = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) => {
            diagnostics.push(Diagnostic {
                error: !fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()),
                message: format!("broken or unreadable path {:?}: {error}", path),
            });
            return;
        }
    };
    if !canonical.is_dir() {
        return;
    }
    // ponytail: bounded recursive depth; iterative traversal if real skill trees exceed 256 levels.
    if active.contains(&canonical) || active.len() >= 256 {
        diagnostics.push(Diagnostic {
            error: false,
            message: format!("cycle or depth limit at {:?} -> {:?}", path, canonical),
        });
        return;
    }
    active.insert(canonical.clone());
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries.collect::<Result<Vec<_>, _>>(),
        Err(error) => Err(error),
    };
    match entries {
        Ok(mut entries) => {
            entries.sort_by_key(|entry| entry.file_name());
            let is_skill = entries.iter().any(|entry| entry.file_name() == "SKILL.md");
            if is_skill {
                if let Some(skill) = skills.get_mut(&canonical) {
                    skill.exposures.push(path.to_owned());
                } else {
                    match skill_name(&canonical) {
                        Ok(name) => {
                            skills.insert(
                                canonical.clone(),
                                Skill {
                                    name,
                                    canonical_dir: canonical.clone(),
                                    exposures: vec![path.to_owned()],
                                },
                            );
                        }
                        Err(message) => diagnostics.push(Diagnostic {
                            error: true,
                            message,
                        }),
                    }
                }
            }
            if !selected_only || !is_skill {
                for entry in entries {
                    if !ignores.iter().any(|pattern| {
                        glob(
                            pattern.as_bytes(),
                            entry.file_name().to_string_lossy().as_bytes(),
                        )
                    }) {
                        walk(
                            &entry.path(),
                            ignores,
                            active,
                            skills,
                            diagnostics,
                            selected_only,
                        );
                    }
                }
            }
        }
        Err(error) => diagnostics.push(Diagnostic {
            error: true,
            message: format!("cannot list {:?}: {error}", path),
        }),
    }
    active.remove(&canonical);
}
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    // Linear wildcard matching, basename only: * and ?; no regex/glob framework.
    let (mut p, mut t, mut star, mut retry) = (0, 0, None, 0);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            retry = t;
        } else if let Some(s) = star {
            p = s + 1;
            retry += 1;
            t = retry;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}
fn skill_name(directory: &Path) -> Result<String, String> {
    let path = directory.join("SKILL.md");
    if !fs::metadata(&path).is_ok_and(|m| m.is_file()) {
        return Err(format!(
            "SKILL.md is not a readable regular file: {:?}",
            path
        ));
    }
    let source = fs::read_to_string(&path).map_err(|e| format!("cannot read {:?}: {e}", path))?;
    let mut name = None;
    let mut lines = source.lines();
    if lines.next() == Some("---") {
        let mut closed = false;
        for line in lines {
            if line == "---" {
                closed = true;
                break;
            }
            if let Some(value) = line.strip_prefix("name:") {
                if name.is_some() {
                    return Err(format!("duplicate frontmatter name in {:?}", path));
                }
                let value = value.trim();
                let value = value
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .or_else(|| value.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                    .unwrap_or(value);
                name = Some(value.to_owned());
            }
        }
        if !closed {
            return Err(format!("unterminated frontmatter in {:?}", path));
        }
    }
    let name = name
        .or_else(|| {
            directory
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
        })
        .ok_or_else(|| format!("no skill name for {:?}", directory))?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
    {
        return Err(format!(
            "invalid skill name in {:?}; expected ASCII letters, digits, _, - or .",
            path
        ));
    }
    Ok(name)
}
