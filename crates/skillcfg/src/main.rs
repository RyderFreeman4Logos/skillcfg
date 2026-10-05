use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, Write},
    path::PathBuf,
    process,
};

use skillcfg_core::{Config, ConfigKey, discovery, render_value};

const USAGE: &str =
    "Usage: skillcfg [--config PATH] get <literal.key> | discover [--root PATH]... [--verbose]";

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
    for diagnostic in &result.diagnostics {
        eprintln!("skillcfg: {}", diagnostic.message);
    }
    if result.diagnostics.iter().any(|d| d.error) {
        return Err(CliError::Failure("discovery failed".to_owned()));
    }
    let mut output = String::new();
    for skill in &result.skills {
        if verbose {
            output.push_str(&format!(
                "{}\t{:?}\t{:?}\n",
                skill.name, skill.canonical_dir, skill.exposures
            ));
        } else {
            output.push_str(&format!("{}\n", skill.name));
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
