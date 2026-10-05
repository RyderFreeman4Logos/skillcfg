use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, Write},
    path::PathBuf,
    process,
};

use skillcfg_core::{
    Config, ConfigKey, discovery, display_path,
    manifest::{Manifest, render_pairs},
    render_value, validate,
};

const USAGE: &str = "Usage: skillcfg [--config PATH] <command> [options]\nCommands: get KEY | get-many KEY... [--format kv|json] | show-skill NAME [--all] [--format kv|json] | discover [--root PATH]... [--verbose] | validate [--root PATH]... [--strict] | validate-skill PATH [--strict] | explain KEY [--root PATH]...";

#[derive(Debug)]
enum CliError {
    Help,
    Usage(String),
    Failure(String),
}

fn main() {
    match execute(env::args_os().skip(1)) {
        Ok(()) => {}
        Err(CliError::Help) => println!("{USAGE}"),
        Err(CliError::Usage(message)) => {
            eprintln!("skillcfg: {message}\n{USAGE}");
            process::exit(2);
        }
        Err(CliError::Failure(message)) => {
            eprintln!("skillcfg: {message}");
            process::exit(1);
        }
    }
}

fn execute(args: impl Iterator<Item = OsString>) -> Result<(), CliError> {
    let mut args = args.peekable();
    let mut explicit_config = None;

    loop {
        match args.next() {
            Some(argument) if argument == OsStr::new("--config") => {
                if explicit_config.is_some() {
                    return Err(CliError::Usage("--config may be specified once".to_owned()));
                }
                let path = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--config requires a path".to_owned()))?;
                let path = PathBuf::from(path);
                if path.as_os_str().is_empty() {
                    return Err(CliError::Usage(
                        "--config requires a non-empty path".to_owned(),
                    ));
                }
                explicit_config = Some(path);
            }
            Some(argument) if argument == OsStr::new("--help") || argument == OsStr::new("-h") => {
                return Err(CliError::Help);
            }
            Some(command) => {
                let command = command
                    .into_string()
                    .map_err(|_| CliError::Usage("command name must be valid UTF-8".to_owned()))?;
                if command == "explain" {
                    return explain(explicit_config, args);
                }
                if command == "validate" || command == "validate-skill" {
                    return validate_command(explicit_config, &command, args);
                }
                if command == "get-many" || command == "show-skill" {
                    return batch(explicit_config, &command, args);
                }
                if command == "discover" {
                    return discover(explicit_config, args);
                }
                if command != "get" {
                    return Err(CliError::Usage(format!("unknown command '{command}'")));
                }
                let key = args
                    .next()
                    .ok_or_else(|| CliError::Usage("get requires a literal dotted key".to_owned()))?
                    .into_string()
                    .map_err(|_| CliError::Usage("config key must be valid UTF-8".to_owned()))?;
                if args.next().is_some() {
                    return Err(CliError::Usage("get accepts exactly one key".to_owned()));
                }
                return get(explicit_config, &key);
            }
            None => return Err(CliError::Usage("a command is required".to_owned())),
        }
    }
}

fn get(explicit_config: Option<PathBuf>, key: &str) -> Result<(), CliError> {
    let key: ConfigKey = key
        .parse::<ConfigKey>()
        .map_err(|error| CliError::Usage(error.to_string()))?;
    let path = config_path(explicit_config)?;
    let config = Config::load(&path).map_err(|error| CliError::Failure(error.to_string()))?;
    let value = config
        .get(&key)
        .map_err(|error| CliError::Failure(error.to_string()))?;
    let rendered = render_value(value);
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    stdout
        .write_all(rendered.as_bytes())
        .map_err(|error| CliError::Failure(format!("cannot write stdout: {error}")))?;
    if !rendered.ends_with('\n') {
        stdout
            .write_all(b"\n")
            .map_err(|error| CliError::Failure(format!("cannot write stdout: {error}")))?;
    }
    Ok(())
}

fn discover(
    explicit_config: Option<PathBuf>,
    mut args: impl Iterator<Item = OsString>,
) -> Result<(), CliError> {
    let mut roots = Vec::new();
    let mut verbose = false;
    while let Some(arg) = args.next() {
        if arg == "--verbose" {
            verbose = true;
        } else if arg == "--root" {
            let root = args
                .next()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| CliError::Usage("--root requires a nonempty path".to_owned()))?;
            roots.push(PathBuf::from(root));
        } else {
            return Err(CliError::Usage(
                "discover accepts --root PATH and --verbose".to_owned(),
            ));
        }
    }
    let path = config_path(explicit_config.clone())?;
    let config =
        if explicit_config.is_some() || env::var_os("SKILLCFG_CONFIG").is_some() || path.exists() {
            Some(Config::load(&path).map_err(|e| CliError::Failure(e.to_string()))?)
        } else {
            None
        };
    let home = PathBuf::from(env::var_os("HOME").unwrap_or_default());
    let (configured, ignores, explicit) =
        discovery::settings(config.as_ref(), &path, &home).map_err(CliError::Failure)?;
    let explicit = explicit || !roots.is_empty();
    if roots.is_empty() {
        roots = configured;
    }
    let result = discovery::discover(&roots, &ignores, explicit);
    report(&result, false)?;
    let mut output = String::new();
    for skill in &result.skills {
        if verbose {
            output.push_str(&format!(
                "{}\t{:?}\t{:?}\tSKILL.md={:?}\tmanifest={:?}\tsymlink_exposures={:?}\n",
                skill.name,
                skill.canonical_dir,
                skill.exposures,
                skill.canonical_dir.join("SKILL.md"),
                skill.canonical_dir.join("skillcfg.toml"),
                skill
                    .exposures
                    .iter()
                    .map(|p| (
                        p,
                        std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink())
                    ))
                    .collect::<Vec<_>>()
            ));
        } else {
            output.push_str(&format!("{}\n", skill.name));
        }
    }
    write_output(&output)
}

fn batch(
    explicit_config: Option<PathBuf>,
    command: &str,
    mut args: impl Iterator<Item = OsString>,
) -> Result<(), CliError> {
    let mut json = false;
    let mut all = false;
    let mut positional = Vec::new();
    let mut roots = Vec::new();
    let mut format_seen = false;
    while let Some(arg) = args.next() {
        if arg == "--format" {
            if format_seen {
                return Err(CliError::Usage("--format may be specified once".to_owned()));
            }
            format_seen = true;
            let format = args
                .next()
                .ok_or_else(|| CliError::Usage("--format requires kv or json".to_owned()))?;
            if format == "json" {
                json = true;
            } else if format != "kv" {
                return Err(CliError::Usage("--format requires kv or json".to_owned()));
            }
        } else if arg == "--all" && command == "show-skill" {
            all = true;
        } else if arg == "--root" && command == "show-skill" {
            roots.push(PathBuf::from(
                args.next()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| CliError::Usage("--root requires a nonempty path".to_owned()))?,
            ));
        } else {
            let arg = arg
                .into_string()
                .map_err(|_| CliError::Usage("argument must be valid UTF-8".to_owned()))?;
            if arg.starts_with('-') {
                return Err(CliError::Usage(format!("unsupported option {arg:?}")));
            }
            positional.push(arg);
        }
    }
    if positional.is_empty() || (command == "show-skill" && positional.len() != 1) {
        return Err(CliError::Usage(format!(
            "{command} has invalid argument count"
        )));
    }
    // Validate all caller keys before reading a configuration file.
    let keys = if command == "get-many" {
        positional
            .iter()
            .map(|s| {
                s.parse::<ConfigKey>()
                    .map_err(|e| CliError::Usage(e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let path = config_path(explicit_config)?;
    let config = Config::load(&path).map_err(|e| CliError::Failure(e.to_string()))?;
    if command == "get-many" {
        let pairs = keys
            .iter()
            .map(|k| {
                config
                    .get(k)
                    .map(|v| (k.as_str(), v))
                    .map_err(|e| CliError::Failure(e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        return write_output(&render_pairs(&pairs, json).map_err(CliError::Failure)?);
    }
    let result = find_skills(&config, &path, roots)?;
    report(&result, false)?;
    let name = &positional[0];
    let skill = result
        .skills
        .iter()
        .find(|s| &s.name == name)
        .ok_or_else(|| CliError::Failure(format!("skill {name:?} not found")))?;
    let manifest =
        Manifest::load(&skill.canonical_dir.join("skillcfg.toml")).map_err(CliError::Failure)?;
    let mut bindings = manifest.visible.iter().collect::<Vec<_>>();
    if all {
        bindings.extend(manifest.opaque.iter());
    }
    bindings.sort_by_key(|(alias, _)| *alias);
    let pairs = bindings
        .into_iter()
        .map(|(alias, b)| {
            config
                .get(&b.key)
                .map(|v| (alias.as_str(), v))
                .map_err(|e| CliError::Failure(e.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    write_output(&render_pairs(&pairs, json).map_err(CliError::Failure)?)
}

fn find_skills(
    config: &Config,
    path: &std::path::Path,
    mut roots: Vec<PathBuf>,
) -> Result<discovery::Discovery, CliError> {
    let home = PathBuf::from(env::var_os("HOME").unwrap_or_default());
    let (configured, ignores, explicit) =
        discovery::settings(Some(config), path, &home).map_err(CliError::Failure)?;
    let explicit = explicit || !roots.is_empty();
    if roots.is_empty() {
        roots = configured;
    }
    Ok(discovery::discover(&roots, &ignores, explicit))
}
fn report(result: &discovery::Discovery, strict: bool) -> Result<(), CliError> {
    for diagnostic in &result.diagnostics {
        eprintln!("skillcfg: {}", diagnostic.message);
    }
    if result.diagnostics.iter().any(|d| d.error || strict) {
        Err(CliError::Failure("discovery failed".to_owned()))
    } else {
        Ok(())
    }
}

fn validate_command(
    explicit_config: Option<PathBuf>,
    command: &str,
    mut args: impl Iterator<Item = OsString>,
) -> Result<(), CliError> {
    let mut strict = false;
    let mut roots = Vec::new();
    let mut skill_path = None;
    while let Some(arg) = args.next() {
        if arg == "--strict" {
            strict = true;
        } else if arg == "--root" && command == "validate" {
            roots.push(PathBuf::from(
                args.next()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| CliError::Usage("--root requires a nonempty path".to_owned()))?,
            ));
        } else if command == "validate-skill"
            && skill_path.is_none()
            && !arg.to_string_lossy().starts_with('-')
        {
            skill_path = Some(PathBuf::from(arg));
        } else {
            return Err(CliError::Usage(format!(
                "unsupported argument for {command}"
            )));
        }
    }
    if command == "validate-skill" && skill_path.is_none() {
        return Err(CliError::Usage(
            "validate-skill requires a skill directory".to_owned(),
        ));
    }
    let path = config_path(explicit_config)?;
    let config = Config::load(&path).map_err(|e| CliError::Failure(e.to_string()))?;
    let single = skill_path
        .as_ref()
        .map(std::fs::canonicalize)
        .transpose()
        .map_err(|e| CliError::Failure(format!("cannot resolve skill directory: {e}")))?;
    let mut result = if let Some(skill_path) = skill_path {
        discovery::discover(&[skill_path], &[], true)
    } else {
        find_skills(&config, &path, roots)?
    };
    if let Some(single) = single {
        result.skills.retain(|s| s.canonical_dir == single);
        if result.skills.is_empty() {
            return Err(CliError::Failure(format!(
                "skill directory {:?} has no readable SKILL.md",
                single
            )));
        }
    }
    for skill in &result.skills {
        result
            .diagnostics
            .extend(validate::analyze(skill, &config).diagnostics);
    }
    report(&result, strict)?;
    write_output("ok\n")
}

fn explain(
    explicit_config: Option<PathBuf>,
    mut args: impl Iterator<Item = OsString>,
) -> Result<(), CliError> {
    let mut key = None;
    let mut roots = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--root" {
            roots.push(PathBuf::from(
                args.next()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| CliError::Usage("--root requires a nonempty path".to_owned()))?,
            ));
        } else if key.is_none() {
            let raw = arg
                .into_string()
                .map_err(|_| CliError::Usage("key must be valid UTF-8".to_owned()))?;
            key = Some(
                raw.parse::<ConfigKey>()
                    .map_err(|e| CliError::Usage(e.to_string()))?,
            );
        } else {
            return Err(CliError::Usage(
                "explain accepts one key and --root PATH".to_owned(),
            ));
        }
    }
    let key = key.ok_or_else(|| CliError::Usage("explain requires a key".to_owned()))?;
    let path = config_path(explicit_config)?;
    let config = Config::load(&path).map_err(|e| CliError::Failure(e.to_string()))?;
    let value = config
        .get(&key)
        .map_err(|e| CliError::Failure(e.to_string()))?;
    let mut result = find_skills(&config, &path, roots)?;
    let analyses = result
        .skills
        .iter()
        .map(|s| (s.name.clone(), validate::analyze(s, &config)))
        .collect::<Vec<_>>();
    for (_, analysis) in &analyses {
        for diagnostic in &analysis.diagnostics {
            result.diagnostics.push(discovery::Diagnostic {
                error: diagnostic.error,
                message: diagnostic.message.clone(),
            });
        }
    }
    report(&result, false)?;
    let resolved = std::fs::canonicalize(&path)
        .map_err(|e| CliError::Failure(format!("cannot resolve config path: {e}")))?;
    let mut output = format!(
        "key={}\nsource={:?}\ncanonical_source={:?}\n",
        key.as_str(),
        path,
        resolved
    );
    output.push_str(&render_pairs(&[("value", value)], false).map_err(CliError::Failure)?);
    let index = validate::reverse_index(&analyses);
    if let Some(references) = index.get(key.as_str()) {
        output.push_str("references:\n");
        for (skill, r) in references {
            output.push_str(&format!(
                "{}\t{}:{}\t{}\n",
                skill,
                display_path(&r.path),
                r.line,
                r.origin
            ));
        }
    } else {
        output.push_str("references: none\n");
    }
    for (skill, analysis) in &analyses {
        for r in &analysis.references {
            if r.key.is_none() {
                output.push_str(&format!(
                    "unknown\t{}\t{}:{}\n",
                    skill,
                    display_path(&r.path),
                    r.line
                ));
            }
        }
    }
    write_output(&output)
}

fn write_output(rendered: &str) -> Result<(), CliError> {
    io::stdout()
        .lock()
        .write_all(rendered.as_bytes())
        .map_err(|error| CliError::Failure(format!("cannot write stdout: {error}")))
}

fn config_path(explicit: Option<PathBuf>) -> Result<PathBuf, CliError> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Some(path) = env::var_os("SKILLCFG_CONFIG") {
        if path.is_empty() {
            return Err(CliError::Failure("SKILLCFG_CONFIG is empty".to_owned()));
        }
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        if path.is_empty() {
            return Err(CliError::Failure("XDG_CONFIG_HOME is empty".to_owned()));
        }
        return Ok(PathBuf::from(path).join("skillcfg/config.toml"));
    }
    let home = env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            CliError::Failure("no config path; pass --config or set SKILLCFG_CONFIG".to_owned())
        })?;
    Ok(PathBuf::from(home).join(".config/skillcfg/config.toml"))
}
