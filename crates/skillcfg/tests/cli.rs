use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let base = std::env::temp_dir();
        loop {
            let path = base.join(format!(
                "skillcfg-cli-{}-{}",
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

    fn write_config(&self, name: &str, value: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(
            &path,
            format!("schema_version = 1\n[demo]\nmessage = {value:?}\n"),
        )
        .unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str], home: &Path, xdg: Option<&Path>, env_config: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_skillcfg"));
    command
        .args(args)
        .env("HOME", home)
        .env_remove("SKILLCFG_CONFIG")
        .env_remove("XDG_CONFIG_HOME");
    if let Some(path) = xdg {
        command.env("XDG_CONFIG_HOME", path);
    }
    if let Some(path) = env_config {
        command.env("SKILLCFG_CONFIG", path);
    }
    command.output().unwrap()
}

fn message(output: Output) -> String {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn get_prints_raw_value_and_missing_key_is_stderr_only() {
    let temp = TempDir::new();
    let config = temp.write_config("config.toml", "hello");
    let config_arg = config.to_str().unwrap();

    let output = run(
        &["--config", config_arg, "get", "demo.message"],
        &temp.0,
        None,
        None,
    );
    assert_eq!(message(output), "hello\n");

    let missing = run(
        &["--config", config_arg, "get", "demo.absent"],
        &temp.0,
        None,
        None,
    );
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("demo.absent"));
}

#[test]
fn missing_and_malformed_configs_are_stderr_only() {
    let temp = TempDir::new();
    let missing = temp.0.join("missing.toml");
    let output = run(
        &["--config", missing.to_str().unwrap(), "get", "demo.message"],
        &temp.0,
        None,
        None,
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&missing.display().to_string()));

    let invalid = temp.0.join("invalid.toml");
    fs::write(&invalid, "schema_version = [").unwrap();
    let output = run(
        &["--config", invalid.to_str().unwrap(), "get", "demo.message"],
        &temp.0,
        None,
        None,
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&invalid.display().to_string()));
    assert!(stderr.contains("invalid TOML"));
}

fn assert_private_error(source: &str, category: &str) {
    let temp = TempDir::new();
    let path = temp.0.join("private.toml");
    fs::write(&path, source).unwrap();
    let output = run(
        &["--config", path.to_str().unwrap(), "get", "text"],
        &temp.0,
        None,
        None,
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(category));
    assert!(stderr.contains(path.to_str().unwrap()));
    assert!(!stderr.contains("SYNTHETIC_PRIVATE_CANARY"), "{stderr}");
}

#[test]
fn parser_diagnostics_do_not_disclose_source() {
    assert_private_error(
        "schema_version = 1\ntext = \"ok\"\nprivate_value = \"SYNTHETIC_PRIVATE_CANARY\" broken\n",
        "invalid TOML",
    );
}

#[test]
fn schema_table_diagnostics_do_not_disclose_values() {
    assert_private_error(
        "schema_version = { private_value = \"SYNTHETIC_PRIVATE_CANARY\" }\ntext = \"ok\"\n",
        "unsupported schema_version",
    );
}

#[test]
fn schema_string_diagnostics_do_not_disclose_values() {
    assert_private_error(
        "schema_version = \"SYNTHETIC_PRIVATE_CANARY\"\ntext = \"ok\"\n",
        "unsupported schema_version",
    );
}

#[test]
fn config_path_precedence_is_cli_then_env_then_xdg_then_home() {
    let temp = TempDir::new();
    let home = temp.0.join("home");
    let xdg = temp.0.join("xdg");
    fs::create_dir_all(home.join(".config/skillcfg")).unwrap();
    fs::create_dir_all(xdg.join("skillcfg")).unwrap();
    let cli = temp.write_config("cli.toml", "cli");
    let env = temp.write_config("env.toml", "env");
    fs::write(
        xdg.join("skillcfg/config.toml"),
        "schema_version = 1\n[demo]\nmessage = \"xdg\"\n",
    )
    .unwrap();
    fs::write(
        home.join(".config/skillcfg/config.toml"),
        "schema_version = 1\n[demo]\nmessage = \"home\"\n",
    )
    .unwrap();

    let with_all = run(&["get", "demo.message"], &home, Some(&xdg), Some(&env));
    assert_eq!(message(with_all), "env\n");

    let cli_output = run(
        &["--config", cli.to_str().unwrap(), "get", "demo.message"],
        &home,
        Some(&xdg),
        Some(&env),
    );
    assert_eq!(message(cli_output), "cli\n");

    let xdg_output = run(&["get", "demo.message"], &home, Some(&xdg), None);
    assert_eq!(message(xdg_output), "xdg\n");

    let home_output = run(&["get", "demo.message"], &home, None, None);
    assert_eq!(message(home_output), "home\n");
}

#[test]
fn help_shaped_option_values_are_not_help_flags() {
    let temp = TempDir::new();
    let config = temp.write_config("config.toml", "hello");
    let mut failures = Vec::new();
    for flag in ["--help", "-h"] {
        let config_dir = TempDir::new();
        config_dir.write_config(flag, "config-value");
        let out = Command::new(env!("CARGO_BIN_EXE_skillcfg"))
            .args(["--config", flag, "get", "demo.message"])
            .current_dir(&config_dir.0)
            .env_clear()
            .output()
            .unwrap();
        assert_eq!(message(out), "config-value\n");
        let skill = temp.0.join(flag);
        fs::create_dir(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: fixture\n---\n").unwrap();
        fs::write(
            skill.join("skillcfg.toml"),
            "schema_version = 1\n[visible]\nmessage = \"demo.message\"\n",
        )
        .unwrap();
        for (tail, expected) in [
            (vec!["discover"], "fixture\n"),
            (vec!["validate"], "ok\n"),
            (vec!["show-skill", "fixture"], "message=hello\n"),
            (vec!["explain", "demo.message"], "value=hello\n"),
        ] {
            for root in [flag.to_owned(), format!("./{flag}")] {
                let mut args = vec!["--config", config.to_str().unwrap()];
                args.extend(&tail);
                args.extend(["--root", &root]);
                let out = Command::new(env!("CARGO_BIN_EXE_skillcfg"))
                    .args(&args)
                    .current_dir(&temp.0)
                    .env_clear()
                    .output()
                    .unwrap();
                let stdout = String::from_utf8_lossy(&out.stdout);
                if !out.status.success() || !stdout.contains(expected) {
                    failures.push(format!("{args:?}: {stdout}; {:?}", out.stderr));
                }
            }
        }
        for command in ["get-many", "show-skill"] {
            let out = run(
                &[
                    "--config",
                    config.to_str().unwrap(),
                    command,
                    "demo.message",
                    "--format",
                    flag,
                ],
                &temp.0,
                None,
                None,
            );
            if out.status.code() != Some(2)
                || !out.stdout.is_empty()
                || !String::from_utf8_lossy(&out.stderr).contains("--format requires kv or json")
            {
                failures.push(format!("{command} --format {flag}: {out:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn every_help_option_succeeds_without_loading_config_or_home() {
    let temp = TempDir::new();
    let missing_config = temp.0.join("must-not-be-read.toml");
    let cases = [
        (
            "",
            "Usage: skillcfg [--config PATH] <COMMAND> [OPTIONS]",
            "List discovered skills from configured or conventional roots.",
        ),
        (
            "get",
            "Usage: skillcfg [--config PATH] get KEY",
            "literal dotted path",
        ),
        (
            "get-many",
            "Usage: skillcfg [--config PATH] get-many KEY... [--format kv|json]",
            "distinct literal dotted keys",
        ),
        (
            "show-skill",
            "Usage: skillcfg [--config PATH] show-skill NAME [--all] [--format kv|json] [--root PATH]...",
            "opaque bindings",
        ),
        (
            "discover",
            "Usage: skillcfg [--config PATH] discover [--root PATH]... [--verbose]",
            "$HOME/.codex/skills",
        ),
        (
            "validate",
            "Usage: skillcfg [--config PATH] validate [--root PATH]... [--strict]",
            "Warnings are non-fatal unless --strict is set.",
        ),
        (
            "validate-skill",
            "Usage: skillcfg [--config PATH] validate-skill PATH [--strict]",
            "without traversing\nconfigured roots",
        ),
        (
            "explain",
            "Usage: skillcfg [--config PATH] explain KEY [--root PATH]...",
            "intentionally displays the requested value",
        ),
    ];

    for (command_name, usage, detail) in cases {
        for help_flag in ["-h", "--help"] {
            let mut args = vec!["--config".to_owned(), missing_config.display().to_string()];
            if !command_name.is_empty() {
                args.push(command_name.to_owned());
            }
            args.push(help_flag.to_owned());
            let output = Command::new(env!("CARGO_BIN_EXE_skillcfg"))
                .args(&args)
                .current_dir(&temp.0)
                .env_clear()
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                output.stderr.is_empty(),
                "{args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(stdout.contains(usage), "{args:?}: {stdout}");
            assert!(
                stdout.contains(detail),
                "{args:?}: missing {detail:?}: {stdout}"
            );
        }
    }
}
