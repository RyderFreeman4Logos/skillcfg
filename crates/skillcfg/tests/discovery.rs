use std::os::unix::fs::symlink;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "skillcfg-discovery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn skill(&self, relative: &str, name: &str) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!("---\nname: {name}\n---\n# Demo\n"),
        )
        .unwrap();
        path
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_skillcfg"))
            .args(args)
            .env("HOME", &self.0)
            .env_remove("SKILLCFG_CONFIG")
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn recursive_multi_root_discovery_deduplicates_with_exposures() {
    let f = Fixture::new();
    let actual = f.skill("canonical/category/deep/review", "review");
    f.skill("canonical/direct", "direct");
    f.skill("canonical", "root-skill");
    f.skill("canonical/target/ignored", "ignored");
    for root in ["codex", "claude", "hermes", "agents"] {
        fs::create_dir(f.0.join(root)).unwrap();
        symlink(&actual, f.0.join(root).join("review")).unwrap();
    }
    symlink(f.0.join("canonical"), f.0.join("category-link")).unwrap();
    let out = f.run(&[
        "discover",
        "--root",
        text(&f.0.join("codex")),
        "--root",
        text(&f.0.join("claude")),
        "--root",
        text(&f.0.join("hermes")),
        "--root",
        text(&f.0.join("agents")),
        "--root",
        text(&f.0.join("category-link")),
        "--verbose",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout.lines().filter(|s| s.starts_with("review\t")).count(),
        1
    );
    assert!(stdout.contains(text(&actual)));
    for root in ["codex", "claude", "hermes", "agents"] {
        assert!(stdout.contains(text(&f.0.join(root).join("review"))));
    }
    assert!(stdout.contains("direct\t"));
    assert!(stdout.contains("root-skill\t"));
    assert!(!stdout.contains("ignored"));
    assert!(out.stderr.is_empty());
}

#[test]
fn broken_cycles_and_collisions_are_deterministic_and_actionable() {
    let f = Fixture::new();
    f.skill("root/a", "same");
    f.skill("root/b", "same");
    symlink("missing", f.0.join("root/broken")).unwrap();
    symlink(".", f.0.join("root/cycle")).unwrap();
    let root = f.0.join("root");
    let args = ["discover", "--root", text(&root), "--verbose"];
    let first = f.run(&args);
    let second = f.run(&args);
    assert_eq!(first.status.code(), Some(1));
    assert_eq!(first.stderr, second.stderr);
    let stderr = String::from_utf8(first.stderr).unwrap();
    for needle in ["collision", "root/a", "root/b", "broken", "cycle"] {
        assert!(stderr.contains(needle), "{stderr}");
    }
}

#[test]
fn configured_relative_and_home_roots_follow_file_links() {
    let f = Fixture::new();
    let actual = f.skill("outside", "linked-file");
    fs::create_dir_all(f.0.join("relative/skill")).unwrap();
    symlink(actual.join("SKILL.md"), f.0.join("relative/skill/SKILL.md")).unwrap();
    f.skill("home-root/fallback", "home");
    let config = f.0.join("config.toml");
    fs::write(
        &config,
        "schema_version = 1\n[discovery]\nroots = [\"relative\", \"~/home-root\"]\n",
    )
    .unwrap();
    let out = f.run(&["--config", text(&config), "discover"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "home\nlinked-file\n"
    );
    let missing = f.run(&["discover", "--root", text(&f.0.join("absent"))]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("absent"));
    assert!(f.run(&["discover"]).status.success());
}
