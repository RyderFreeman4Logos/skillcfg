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
#[test]
fn batch_output_is_ordered_atomic_and_json_parseable() {
    let f = Fixture::new();
    let config = f.0.join("config.toml");
    fs::write(&config, "schema_version = 1\n[values]\na = \"hello\"\nz = 7\nmultiline = \"line\\nnext\"\narray = [1, true]\n").unwrap();
    let out = f.run(&[
        "--config",
        text(&config),
        "get-many",
        "values.z",
        "values.a",
        "values.multiline",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "values.z=7\nvalues.a=hello\nvalues.multiline=\"line\\nnext\"\n"
    );
    let out = f.run(&[
        "--config",
        text(&config),
        "get-many",
        "values.a",
        "values.array",
        "--format",
        "json",
    ]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "{\"values.a\":\"hello\",\"values.array\":[1,true]}\n"
    );
    let out = f.run(&[
        "--config",
        text(&config),
        "get-many",
        "values.a",
        "values.absent",
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("hello"));
}

#[test]
fn show_skill_defaults_to_visible_with_optional_linked_manifest() {
    let f = Fixture::new();
    let skill = f.skill("root/review", "review");
    f.skill("root/no-manifest", "no-manifest");
    let manifest = f.0.join("manifest.toml");
    fs::write(&manifest, "schema_version = 1\n[visible]\nstyle = \"review.style\"\n[opaque]\nmodel = \"review.model\"\n").unwrap();
    symlink(&manifest, skill.join("skillcfg.toml")).unwrap();
    let config = f.0.join("config.toml");
    fs::write(&config, "schema_version = 1\n[discovery]\nroots = [\"root\"]\n[review]\nstyle = \"careful\"\nmodel = \"SYNTHETIC_PRIVATE_CANARY\"\n").unwrap();
    let out = f.run(&["--config", text(&config), "show-skill", "review"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "style=careful\n");
    assert!(out.stderr.is_empty());
    let out = f.run(&[
        "--config",
        text(&config),
        "show-skill",
        "review",
        "--all",
        "--format",
        "json",
    ]);
    assert!(out.status.success());
    assert!(
        String::from_utf8(out.stdout)
            .unwrap()
            .contains("SYNTHETIC_PRIVATE_CANARY")
    );
    let out = f.run(&["--config", text(&config), "show-skill", "no-manifest"]);
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    for source in [
        "schema_version = 1\n[visible]\nx = [",
        "schema_version = 1\n[visible]\nx = \"review.style\"\n[opaque]\nx = \"review.model\"\n",
    ] {
        fs::write(&manifest, source).unwrap();
        let out = f.run(&["--config", text(&config), "show-skill", "review"]);
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&out.stderr).contains("SYNTHETIC_PRIVATE_CANARY"));
    }
}

#[test]
fn validation_reports_literal_locations_and_dynamic_unknowns_without_values() {
    let f = Fixture::new();
    let skill = f.skill("skill", "demo");
    let config = f.0.join("config.toml");
    fs::write(
        &config,
        "schema_version = 1\n[values]\nok = \"SYNTHETIC_PRIVATE_CANARY\"\n",
    )
    .unwrap();
    fs::write(
        skill.join("skillcfg.toml"),
        "schema_version = 1\n[opaque]\nmodel = \"values.ok\"\n",
    )
    .unwrap();
    fs::create_dir(skill.join("scripts")).unwrap();
    let script = skill.join("scripts/run");
    fs::write(&script, "# skillcfg get nonexistent.comment\necho 'skillcfg get nonexistent.text'\nmodel=\"$(skillcfg get values.ok)\"\nskillcfg get values.typo\nskillcfg get \"$prefix.dynamic\"\n").unwrap();
    let out = f.run(&["--config", text(&config), "validate-skill", text(&skill)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8(out.stderr).unwrap();
    for needle in ["scripts/run", ":4", "values.typo", "unverifiable"] {
        assert!(stderr.contains(needle), "{stderr}");
    }
    for needle in [
        "SYNTHETIC_PRIVATE_CANARY",
        "nonexistent.comment",
        "nonexistent.text",
        "prefix.dynamic",
    ] {
        assert!(!stderr.contains(needle), "{stderr}");
    }
    fs::write(&script, "skillcfg get 'values.ok'\n").unwrap();
    let out = f.run(&["--config", text(&config), "validate-skill", text(&skill)]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "ok\n");
    fs::write(
        skill.join("skillcfg.toml"),
        "schema_version = 1\n[visible]\nmissing = \"values.absent\"\n",
    )
    .unwrap();
    let out = f.run(&["--config", text(&config), "validate-skill", text(&skill)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("skillcfg.toml:3"));
    assert!(String::from_utf8_lossy(&out.stderr).contains("values.absent"));
}

#[test]
fn global_validation_strict_links_aliases_and_single_skill_isolation() {
    let f = Fixture::new();
    let skill = f.skill("root/a", "a");
    let other = f.skill("root/b", "b");
    let config = f.0.join("config.toml");
    fs::write(
        &config,
        "schema_version = 1\n[discovery]\nroots = [\"root\"]\n",
    )
    .unwrap();
    symlink("gone", other.join("broken")).unwrap();
    let out = f.run(&[
        "--config",
        text(&config),
        "validate-skill",
        text(&skill),
        "--strict",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = f.run(&["--config", text(&config), "validate", "--strict"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("broken"));
    fs::write(
        skill.join("skillcfg.toml"),
        "schema_version = 1\n[visible]\nx = \"schema_version\"\nx = \"schema_version\"\n",
    )
    .unwrap();
    let out = f.run(&["--config", text(&config), "validate-skill", text(&skill)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid TOML"));
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
