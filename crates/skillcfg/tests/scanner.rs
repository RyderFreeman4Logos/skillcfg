use std::{fs, process::Command};

#[test]
fn scanner_lexical_class_matrix_through_validate_and_explain() {
    let root = std::env::temp_dir().join(format!("skillcfg-scanner-class-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let skill = root.join("review");
    fs::create_dir_all(skill.join("scripts")).unwrap();
    fs::write(skill.join("SKILL.md"), "---\nname: review\n---\n").unwrap();
    let config = root.join("config.toml");
    fs::write(
        &config,
        "schema_version=1\n[values]\nok=\"public\"\nprivate=\"SYNTHETIC_PRIVATE_CANARY\"\n",
    )
    .unwrap();
    type ExpectedReference<'a> = (usize, Option<&'a str>);
    type ScannerCase<'a> = (&'a str, &'a str, &'a [ExpectedReference<'a>]);
    let cases: &[ScannerCase<'_>] = &[
        (
            "single multiline data",
            "printf '%s\\n' '\nskillcfg get values.missing\n'\n",
            &[],
        ),
        (
            "double multiline data",
            "printf \"%s\\n\" \"\nskillcfg get values.missing\n\"\n",
            &[],
        ),
        (
            "continued argument",
            "printf \"%s\\n\" \\\nskillcfg get values.missing\n",
            &[],
        ),
        (
            "backtick multiline data",
            "x=`printf '%s' '\nskillcfg get values.missing\n'`\n",
            &[],
        ),
        (
            "operator comment",
            ":;# $(skillcfg get values.missing)\n",
            &[],
        ),
        (
            "pipe comment",
            ": |# $(skillcfg get values.missing)\ncat\n",
            &[],
        ),
        (
            "substitution comment boundary",
            "echo $(printf x # ) ; skillcfg get values.missing\n)\n",
            &[],
        ),
        (
            "arithmetic is not command",
            "echo \"$((skillcfg get values.missing))\"\n",
            &[],
        ),
        (
            "two queued heredocs",
            "cat <<A <<B\nA\nskillcfg get values.missing\nB\n",
            &[(1, None), (1, None)],
        ),
        (
            "escaped delimiter",
            "cat <<\\EOF\nskillcfg get values.missing\nEOF\n",
            &[(1, None)],
        ),
        (
            "literal dollar delimiter",
            "cat <<\"$DELIM\"\nskillcfg get values.missing\n$DELIM\n",
            &[(1, None)],
        ),
        (
            "empty delimiter",
            "cat <<''\nskillcfg get values.missing\n\n",
            &[(1, None)],
        ),
        (
            "tab strip",
            "cat <<-DOC\n\tskillcfg get values.missing\n\tDOC\nskillcfg get values.ok\n",
            &[(1, None), (4, Some("values.ok"))],
        ),
        (
            "tab strip preserves spaces",
            "cat <<-DOC\n DOC\nskillcfg get values.missing\nDOC\n",
            &[(1, None)],
        ),
        (
            "heredoc command suffix",
            "cat <<DOC; skillcfg get values.ok\nbody\nDOC\n",
            &[(1, None), (1, Some("values.ok"))],
        ),
        (
            "here string suffix",
            "cat <<<data; skillcfg get values.ok\n",
            &[(1, None), (1, Some("values.ok"))],
        ),
        (
            "assignment prefix",
            "MODE=demo skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "command prefix",
            "command skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "backtick prefix",
            "x=`command skillcfg get \"$DYNAMIC\"`\n",
            &[(1, None)],
        ),
        (
            "env prefix",
            "env skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "exec prefix",
            "exec skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "sudo prefix",
            "sudo skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "time prefix",
            "time skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        ("! prefix", "! skillcfg get \"$DYNAMIC\"\n", &[(1, None)]),
        (
            "grouped heredoc",
            "(cat <<DOC)\nskillcfg get values.missing\nDOC\nskillcfg get values.ok\n",
            &[(1, None), (4, Some("values.ok"))],
        ),
        (
            "leading redirection",
            ">out skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "if then positions",
            "if skillcfg get \"$DYNAMIC\"; then skillcfg get values.ok; fi\n",
            &[(1, None), (1, None)],
        ),
        (
            "plain literal",
            "skillcfg get values.ok\n",
            &[(1, Some("values.ok"))],
        ),
        (
            "quoted literal",
            "skillcfg get 'values.ok'\n",
            &[(1, Some("values.ok"))],
        ),
        (
            "continued literal",
            "skillcfg \\\nget values.ok\n",
            &[(1, Some("values.ok"))],
        ),
        (
            "multiline substitution",
            "echo \"$(\nskillcfg get values.ok\n)\"\n",
            &[(2, Some("values.ok"))],
        ),
        (
            "nested literal",
            "echo \"$(printf %s \"$(skillcfg get values.ok)\")\"\n",
            &[(1, Some("values.ok"))],
        ),
        (
            "quoted nested data",
            "echo \"$(printf '%s' '$(skillcfg get values.missing)')\"\n",
            &[],
        ),
        (
            "backtick original",
            "x=`skillcfg get values.missing`\n",
            &[(1, None)],
        ),
        (
            "ordinary argument",
            "printf %s skillcfg get values.missing\n",
            &[],
        ),
        (
            "wrapper data",
            "echo 'command skillcfg get values.missing'\n",
            &[],
        ),
        (
            "hash within word",
            "echo foo#$(skillcfg get values.ok)\n",
            &[(1, Some("values.ok"))],
        ),
        (
            "escaped hash",
            "echo \\#$(skillcfg get values.ok)\n",
            &[(1, Some("values.ok"))],
        ),
        (
            "quoted hash",
            "echo \"#$(skillcfg get values.ok)\"\n",
            &[(1, Some("values.ok"))],
        ),
        (
            "simple heredoc",
            "cat <<'DOC'\nskillcfg get values.missing\nDOC\nskillcfg get values.ok\n",
            &[(1, None), (4, Some("values.ok"))],
        ),
        (
            "unclosed quote",
            "echo '\nskillcfg get values.missing\n",
            &[(1, None)],
        ),
    ];
    let mut failures = Vec::new();
    for (name, source, expected) in cases {
        let actual: Vec<_> = skillcfg_core::validate::scan_source(source)
            .into_iter()
            .map(|(line, key)| (line, key.map(|k| k.as_str().to_owned())))
            .collect();
        let expected: Vec<_> = expected
            .iter()
            .map(|(line, key)| (*line, key.map(str::to_owned)))
            .collect();
        if actual != expected {
            failures.push(format!("{name}: refs {actual:?} != {expected:?}"));
        }
        fs::write(skill.join("scripts/run"), source).unwrap();
        for command in ["validate", "explain"] {
            let mut args = vec!["--config", config.to_str().unwrap(), command];
            if command == "explain" {
                args.push("values.ok");
            }
            args.extend(["--root", skill.to_str().unwrap()]);
            let out = Command::new(env!("CARGO_BIN_EXE_skillcfg"))
                .args(&args)
                .env_remove("SKILLCFG_CONFIG")
                .output()
                .unwrap();
            let stdout = String::from_utf8(out.stdout).unwrap();
            let stderr = String::from_utf8(out.stderr).unwrap();
            let unknown = expected.iter().any(|(_, key)| key.is_none());
            let lines: Vec<_> = expected
                .iter()
                .filter_map(|(line, key)| (key.as_deref() == Some("values.ok")).then_some(*line))
                .collect();
            let rows: Vec<_> = stdout
                .lines()
                .filter(|row| row.ends_with("\tscript"))
                .collect();
            if !out.status.success()
                || stderr.contains("values.missing")
                || (command == "explain"
                    && (rows.len() != lines.len()
                        || lines
                            .iter()
                            .any(|line| !stdout.contains(&format!("scripts/run:{line}\tscript")))
                        || stdout.contains("unknown\t") != unknown))
                || (command == "validate" && stderr.contains("unverifiable") != unknown)
                || (stdout.clone() + &stderr).contains("SYNTHETIC_PRIVATE_CANARY")
            {
                failures.push(format!("{name}: {command}: {stdout} / {stderr}"));
            }
            if command == "validate" {
                let out = Command::new(env!("CARGO_BIN_EXE_skillcfg"))
                    .args(&args)
                    .arg("--strict")
                    .env_remove("SKILLCFG_CONFIG")
                    .output()
                    .unwrap();
                if out.status.success() != expected.is_empty() {
                    failures.push(format!(
                        "{name}: strict must reject unknown/undeclared references"
                    ));
                }
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
