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
