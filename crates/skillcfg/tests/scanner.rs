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
            "named descriptor prefix",
            "{fd}>/dev/null skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "quoted descriptor data",
            "\"2\">/dev/null skillcfg get values.missing\n",
            &[],
        ),
        (
            "separated descriptor data",
            "2 >/dev/null skillcfg get values.missing\n",
            &[],
        ),
        (
            "quoted reserved word data",
            "\"if\" skillcfg get values.missing\n",
            &[],
        ),
        (
            "arithmetic command data",
            "((skillcfg get values.missing))\n",
            &[],
        ),
        (
            "arithmetic active substitution",
            "(( $(skillcfg get values.ok) ))\n",
            &[(1, None)],
        ),
        (
            "indexed array elements data",
            "args=([0]=skillcfg [1]=get [2]=values.missing)\n",
            &[],
        ),
        (
            "compound suffix line",
            "args=(skillcfg get values.missing)\nskillcfg get values.ok\n",
            &[(2, Some("values.ok"))],
        ),
        ("array literal data", "args=(skillcfg get values.ok)\n", &[]),
        (
            "array missing data",
            "args=(skillcfg get values.missing)\n",
            &[],
        ),
        (
            "array multiline quoted data",
            "args=(\n\"skillcfg\" \"get\" \"values.missing\"\n)\n",
            &[],
        ),
        (
            "compound append data",
            "args+=(skillcfg get values.missing)\n",
            &[],
        ),
        (
            "indexed compound data",
            "args[0]=(skillcfg get values.missing)\n",
            &[],
        ),
        (
            "declaration array data",
            "declare -a args=(skillcfg get values.missing)\n",
            &[],
        ),
        (
            "compound data after assignment",
            "other=x args=(skillcfg get values.missing)\n",
            &[],
        ),
        (
            "array active substitution",
            "args=(\"$(skillcfg get values.ok)\")\n",
            &[(1, Some("values.ok"))],
        ),
        (
            "subshell positive",
            "(skillcfg get values.ok)\n",
            &[(1, Some("values.ok"))],
        ),
        (
            "multiline subshell",
            "(\nskillcfg get values.ok\n)\n",
            &[(2, Some("values.ok"))],
        ),
        (
            "indexed assignment prefix",
            "args[0]=x skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "quoted assignment data",
            "\"args=x\" skillcfg get values.missing\n",
            &[],
        ),
        (
            "env option operand",
            "env -u UNUSED skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "command option operand",
            "command -x UNUSED skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "exec option operand",
            "exec -a NAME skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "sudo option operand",
            "sudo -u USER skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "time option operand",
            "time -o OUTPUT skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "leading fd",
            "2>/dev/null skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "leading duplicate fd",
            "2>&1 skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "leading fd newline",
            "2>/dev/null \\\n skillcfg get \"$DYNAMIC\"\n",
            &[(2, None)],
        ),
        (
            "escaped command",
            "\\skillcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "escaped command literal",
            "\\skillcfg get values.missing\n",
            &[(1, None)],
        ),
        (
            "mixed escaped command",
            "sk\\illcfg get \"$DYNAMIC\"\n",
            &[(1, None)],
        ),
        (
            "quoted command literal",
            "\"skillcfg\" get values.ok\n",
            &[(1, Some("values.ok"))],
        ),
        (
            "brace group",
            "{ skillcfg get \"$DYNAMIC\"; }\n",
            &[(1, None)],
        ),
        (
            "brace multiline",
            "{\n skillcfg get \"$DYNAMIC\";\n}\n",
            &[(2, None)],
        ),
        (
            "quoted brace data",
            "\"{\" skillcfg get values.missing\n",
            &[],
        ),
        (
            "escaped brace data",
            "\\{ skillcfg get values.missing\n",
            &[],
        ),
        (
            "wrapper ordinary quoted data",
            "env printf \"%s\" \"skillcfg get values.missing\"\n",
            &[],
        ),
        (
            "fd quoted data",
            "2>/dev/null printf \"%s\" \"skillcfg get values.missing\"\n",
            &[],
        ),
        (
            "brace quoted data",
            "{ printf \"%s\" \"skillcfg get values.missing\"; }\n",
            &[],
        ),
        (
            "escaped data",
            "printf \"%s\" \\skillcfg get values.missing\n",
            &[],
        ),
        (
            "comment roles",
            "# env -u UNUSED \\skillcfg get \"$DYNAMIC\"\n",
            &[],
        ),
        (
            "unknown privacy line",
            "\n env -u UNUSED skillcfg get \"$SYNTHETIC_PRIVATE_EXPRESSION\"\n",
            &[(2, None)],
        ),
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
        for command in ["validate", "validate-skill", "explain"] {
            let mut args = vec!["--config", config.to_str().unwrap(), command];
            if command == "explain" {
                args.push("values.ok");
            }
            if command == "validate-skill" {
                args.push(skill.to_str().unwrap());
            } else {
                args.extend(["--root", skill.to_str().unwrap()]);
            }
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
                || (command != "explain" && stderr.contains("unverifiable") != unknown)
                || (stdout.clone() + &stderr).contains("SYNTHETIC_PRIVATE_CANARY")
                || (stdout.clone() + &stderr).contains("SYNTHETIC_PRIVATE_EXPRESSION")
            {
                failures.push(format!("{name}: {command}: {stdout} / {stderr}"));
            }
            if command != "explain" {
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
