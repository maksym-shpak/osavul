use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    run_in(None, args)
}

fn run_in(dir: Option<&std::path::Path>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
    cmd.args(args);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    cmd.output().expect("не вдалося запустити osavul")
}

fn assert_usage_error(args: &[&str]) {
    let out = run(args);
    assert_eq!(out.status.code(), Some(2), "аргументи: {args:?}");
    assert!(out.stdout.is_empty(), "stdout має бути порожнім: {args:?}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("osavul --help"), "stderr: {stderr:?}");
}

#[test]
fn help_is_stable_and_lists_all_invocations() {
    let first = run(&["--help"]);
    let second = run(&["--help"]);
    for out in [&first, &second] {
        assert_eq!(out.status.code(), Some(0));
        assert!(out.stderr.is_empty());
    }
    let text = String::from_utf8(first.stdout.clone()).unwrap();
    assert!(text.contains("--help") && text.contains("--version"));
    assert!(text.contains("check <файл.rs>"));
    assert!(text.contains("osavul complexity <ревізія>"));
    assert!(text.contains("osavul complexity --path <файл-або-тека>"));
    assert!(text.contains("Git не потрібен"));
    assert!(text.contains("Коди завершення complexity"));
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn version_prints_package_name_and_version() {
    let out = run(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
    let expected = format!("{} {}\n", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    assert_eq!(out.stdout, expected.as_bytes());
}

#[test]
fn no_arguments_is_usage_error() {
    assert_usage_error(&[]);
}

#[test]
fn check_without_path_is_usage_error() {
    assert_usage_error(&["check"]);
}

#[test]
fn extra_argument_is_usage_error() {
    assert_usage_error(&["--version", "extra"]);
    assert_usage_error(&["check", "a.rs", "extra"]);
    assert_usage_error(&["complexity", "HEAD", "extra"]);
    assert_usage_error(&["complexity", "--path", "a.ts", "extra"]);
    assert_usage_error(&["complexity", "--path=a.ts", "a.ts"]);
    assert_usage_error(&["complexity", "a.ts", "--path"]);
}

#[test]
fn complexity_without_revision_is_usage_error() {
    assert_usage_error(&["complexity"]);
}

fn temp_file(name: &str, content: &[u8]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("check-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, content).unwrap();
    path
}

/// Повертає (код, stdout) і перевіряє порожній stderr для кодів 0 та 1.
fn check_src(name: &str, src: &str) -> (i32, String) {
    let path = temp_file(name, src.as_bytes());
    let p = path.to_str().unwrap();
    let out = run(&["check", p]);
    let code = out.status.code().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    if code <= 1 {
        assert!(out.stderr.is_empty());
    }
    (code, stdout.replace(p, "P"))
}

fn assert_check_error(args: &[&str]) {
    let out = run(args);
    assert_eq!(out.status.code(), Some(2), "{args:?}");
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
}

#[test]
fn corpus_matches_expected_tsv() {
    let dir = "tests/fixtures/empty-rust-test";
    let tsv = fs::read_to_string(format!("{dir}/expected.tsv")).unwrap();
    let mut rows = tsv.lines();
    assert_eq!(rows.next(), Some("path\tpair\tlines"));
    let mut count = 0;
    for row in rows {
        let f: Vec<&str> = row.split('\t').collect();
        let path = format!("{dir}/{}", f[0]);
        let (code, expected) = match f[2] {
            "[]" => (0, String::new()),
            l => (1, format!("{path}:{}: empty-rust-test\n", &l[1..l.len() - 1])),
        };
        let first = run(&["check", &path]);
        let second = run(&["check", &path]);
        for out in [&first, &second] {
            assert_eq!(out.status.code(), Some(code), "{path}");
            assert_eq!(out.stdout, expected.as_bytes(), "{path}");
            assert!(out.stderr.is_empty(), "{path}");
        }
        count += 1;
    }
    assert_eq!(count, 6);
}

#[test]
fn two_findings_in_order_with_crlf_and_multiline_attribute() {
    let src = "// c\r\n#[test]\r\nfn a() {}\r\n\r\n#[\r\n  test\r\n]\r\n/* x\r\n*/ pub(crate)\r\nfn b() { /* a /* nested */ b */ // t\r\n}\r\n";
    let path = temp_file("any_name.txt", src.as_bytes());
    // Робочий каталог — тимчасовий, тож тест не залежить від розміщення CARGO_TARGET_DIR.
    let p = format!("./{}", path.file_name().unwrap().to_str().unwrap());
    let out = run_in(path.parent(), &["check", &p]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout, format!("{p}:3: empty-rust-test\n{p}:10: empty-rust-test\n").as_bytes());
    assert!(out.stderr.is_empty());
}

#[test]
fn bom_and_shebang_keep_physical_lines() {
    let (code, out) = check_src("bom.rs", "\u{feff}#!/usr/bin/env x\n#[test]\nfn a() {}\n");
    assert_eq!((code, out.as_str()), (1, "P:3: empty-rust-test\n"));
}

#[test]
fn embedded_modules_count_but_other_items_do_not() {
    let src = "mod m {\n    #[test] pub(crate) fn a() {}\n}\n#[cfg(test)]\nmod t {\n    #[test]\n    fn b() {}\n}\n\
        #[test] async fn c() {}\n#[test] const fn d() {}\n#[test] unsafe fn e() {}\n#[test] extern \"C\" fn f() {}\n\
        struct S;\nimpl S { #[test] fn g() {} }\nfn h() { #[test] fn i() {} }\n";
    let (code, out) = check_src("mods.rs", src);
    assert_eq!((code, out.as_str()), (1, "P:2: empty-rust-test\nP:7: empty-rust-test\n"));
}

#[test]
fn lookalikes_in_literals_comments_and_macros_do_not_match() {
    let src = r####"
const A: &str = "#[test] fn a() {}";
const B: &str = r#"#[test] fn b() {}"#;
const C: &[u8] = br##"#[test] fn c() {}"##;
// #[test] fn d() {}
/* #[test] fn e() {} /* #[test] fn f() {} */ */
macro_rules! m { () => { #[test] fn g() {} }; }
m! { #[test] fn h() {} }
m!(#[test] fn i() {});
fn lt<'a>(_: &'a str) -> char { '{' }
"####;
    assert_eq!(check_src("lookalike.rs", src), (0, String::new()));
}

#[test]
fn extra_attributes_and_non_empty_bodies_do_not_match() {
    for item in [
        "#[test]\n#[ignore]\nfn a() {}",
        "#[test]\n#[should_panic]\nfn a() {}",
        "#[cfg(test)]\n#[test]\nfn a() {}",
        "/// doc\n#[test]\nfn a() {}",
        "#[test]\n#[test]\nfn a() {}",
        "#[test]\nfn a() { //! inner\n}",
        "#[test]\nfn a() { ; }",
        "#[test]\nfn a() { helper() }",
        "#[test]\nfn a() { assert!(true); }",
        "#[inline]\nfn a() {}",
        "fn helper() {}\n#[test]\nfn a() { helper(); }",
    ] {
        assert_eq!(check_src("attrs.rs", item), (0, String::new()), "{item}");
    }
}

#[test]
fn signature_macros_and_unicode_names_and_multiline_inner_attribute() {
    let cases = [
        ("#[test]\nfn a() -> unit! {} { helper() }", (0, "")),
        ("#[test]\nfn a() -> as_type! { () } {}", (1, "P:2: empty-rust-test\n")),
        ("#[test]\nfn a() -> ! {}", (1, "P:2: empty-rust-test\n")),
        ("mod परीक्षा {\n#[test]\nfn a() {}\n}", (1, "P:3: empty-rust-test\n")),
        ("#!\n[allow(dead_code)]\n#[test]\nfn a() {}\n", (1, "P:4: empty-rust-test\n")),
        ("#! /* c */\n// d\n[allow(dead_code)]\n#[test]\nfn a() {}\n", (1, "P:5: empty-rust-test\n")),
    ];
    for (src, (code, out)) in cases {
        assert_eq!(check_src("sig.rs", src), (code, out.to_string()), "{src}");
    }
}

#[test]
fn late_expression_error_leaves_no_partial_report() {
    let path = temp_file("late.rs", b"#[test]\nfn a() {}\nfn b() { let x = (1 +); }\n");
    assert_check_error(&["check", path.to_str().unwrap()]);
}

#[test]
fn external_module_and_unexpanded_macro_need_nothing_else() {
    let src = "mod missing;\ncompile_error!(\"no\");\nfn typed() -> u8 { \"x\" }\n#[test]\nfn a() {}\n";
    assert_eq!(check_src("ext.rs", src), (1, "P:5: empty-rust-test\n".into()));
}

#[test]
fn read_errors_exit_with_two() {
    assert_check_error(&["check", "tests/fixtures/no-such-file.rs"]);
    let path = temp_file("bad_utf8.rs", &[0xff, 0xfe, b'f']);
    assert_check_error(&["check", path.to_str().unwrap()]);
}


const FIXTURES: &str = "tests/fixtures/ts-cyclomatic";

fn isolate(cmd: &mut Command, home: &Path) {
    for (k, _) in std::env::vars_os() {
        if k.to_str().is_some_and(|k| k.starts_with("GIT_")) {
            cmd.env_remove(k);
        }
    }
    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
}

struct Repo {
    root: PathBuf,
    home: PathBuf,
}

impl Repo {
    fn new(name: &str) -> Repo {
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("cx-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("repo");
        let home = base.join("home");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&home).unwrap();
        let repo = Repo { root, home };
        repo.git(&["init", "-q", "-b", "main"]);
        repo
    }

    fn git(&self, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        cmd.args(args).current_dir(&self.root);
        isolate(&mut cmd, &self.home);
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn write(&self, path: &str, content: &[u8]) {
        let full = self.root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, content).unwrap();
    }

    fn commit(&self, msg: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", msg]);
        self.git(&["rev-parse", "HEAD"])
    }

    fn run(&self, dir: &Path, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
        cmd.args(args).current_dir(dir);
        isolate(&mut cmd, &self.home);
        cmd.output().unwrap()
    }

    fn complexity(&self, rev: &str) -> Output {
        self.run(&self.root, &["complexity", rev])
    }

    fn loose_object(&self, oid: &str) -> PathBuf {
        self.root.join(".git/objects").join(&oid[..2]).join(&oid[2..])
    }
}

fn json_of(out: &Output) -> serde_json::Value {
    assert!(out.stdout.ends_with(b"\n"));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn pairs(v: &serde_json::Value) -> Vec<(String, serde_json::Value)> {
    v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["path"].as_str().unwrap().to_string(), f["cyclomatic"].clone()))
        .collect()
}

const IF_ELSE: &str = "export function f(x: number) { if (x) { return 1; } else { return 2; } }\n";

#[test]
fn corpus_pair_matches_expected_scores_through_cli() {
    let repo = Repo::new("corpus");
    let tsv = fs::read_to_string(format!("{FIXTURES}/expected.tsv")).unwrap();
    let mut rows = tsv.lines();
    assert_eq!(rows.next(), Some("path\tpair\tcyclomatic"));
    let mut expected = Vec::new();
    for row in rows {
        let f: Vec<&str> = row.split('\t').collect();
        repo.write(f[0], &fs::read(format!("{FIXTURES}/{}", f[0])).unwrap());
        expected.push((f[0].to_string(), serde_json::json!(f[2].parse::<u64>().unwrap())));
    }
    assert_eq!(expected.len(), 2);
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    let sha = repo.commit("corpus");
    let first = repo.complexity(&sha);
    let second = repo.complexity(&sha);
    for out in [&first, &second] {
        assert_eq!(out.status.code(), Some(0));
        assert!(out.stderr.is_empty());
    }
    assert_eq!(first.stdout, second.stdout);
    let v = json_of(&first);
    assert_eq!(pairs(&v), expected);
    assert_eq!(v["total"], 3);
    assert_eq!(v["complete"], true);
    assert_eq!(v["sha"], sha.as_str());
}

#[test]
fn old_commit_inventory_ignores_later_and_local_changes() {
    let repo = Repo::new("inventory");
    repo.write("a.ts", IF_ELSE.as_bytes());
    repo.write("dir/a.ts", IF_ELSE.as_bytes());
    repo.write("src/deep/x.test.ts", b"function t() { return 1; }\n");
    repo.write("types.d.ts", b"export declare function g(): void;\n");
    repo.write("run.ts", b"function r() { return 1; }\n");
    for skipped in ["U.TS", "x.tsx", "x.mts", "x.cts"] {
        repo.write(skipped, IF_ELSE.as_bytes());
    }
    std::os::unix::fs::symlink("a.ts", repo.root.join("link.ts")).unwrap();
    repo.git(&["update-index", "--add", "--cacheinfo", "160000,1111111111111111111111111111111111111111,sub.ts"]);
    repo.git(&["add", "-A"]);
    repo.git(&["update-index", "--chmod=+x", "run.ts"]);
    repo.git(&["commit", "-q", "-m", "one"]);
    let old = repo.git(&["rev-parse", "HEAD"]);
    let old_stdout = repo.complexity(&old).stdout;

    repo.write("a.ts", b"const changed = 1;\n");
    repo.write("later.ts", b"function l() {}\n");
    repo.commit("two");
    repo.write("a.ts", b"function staged() { return 1; }\n");
    repo.git(&["add", "a.ts"]);
    repo.write("a.ts", b"function worktree() { if (a) {} if (b) {} }\n");
    repo.write("untracked.ts", b"function u() {}\n");

    let expected = vec![
        ("a.ts", 2),
        ("dir/a.ts", 2),
        ("run.ts", 1),
        ("src/deep/x.test.ts", 1),
        ("types.d.ts", 0),
    ];
    let expected: Vec<(String, serde_json::Value)> =
        expected.into_iter().map(|(p, n)| (p.to_string(), serde_json::json!(n))).collect();
    for rev in [old.as_str(), &old[..10], "main~1"] {
        let out = repo.run(&repo.root.join("src/deep"), &["complexity", rev]);
        assert_eq!(out.status.code(), Some(0), "{rev}");
        assert!(out.stderr.is_empty());
        let v = json_of(&out);
        assert_eq!(v["sha"], old.as_str());
        assert_eq!(pairs(&v), expected, "{rev}");
        assert_eq!(v["total"], 6);
        assert_eq!(out.stdout, old_stdout);
    }

    // Локальний звіт читає диск: змінений і невідстежуваний файли видно, індекс не впливає.
    let local = repo.run(&repo.root, &["complexity", "--path", "."]);
    assert_eq!(local.status.code(), Some(0));
    assert!(local.stderr.is_empty());
    let v = json_of(&local);
    assert!(v.as_object().unwrap().contains_key("sha"));
    assert!(v["sha"].is_null());
    assert_eq!(v["scope"]["input"], "directory");
    let expected: Vec<(String, serde_json::Value)> = [
        ("a.ts", 3),
        ("dir/a.ts", 2),
        ("later.ts", 1),
        ("run.ts", 1),
        ("src/deep/x.test.ts", 1),
        ("types.d.ts", 0),
        ("untracked.ts", 1),
    ]
    .into_iter()
    .map(|(p, n)| (p.to_string(), serde_json::json!(n)))
    .collect();
    assert_eq!(pairs(&v), expected);
    assert_eq!(v["total"], 9);
    assert_eq!(repo.complexity(&old).stdout, old_stdout);
}

#[test]
fn single_operand_with_leading_dash_is_a_revision() {
    let repo = Repo::new("dash");
    repo.write("a.ts", b"function a() {}\n");
    let sha = repo.commit("one");
    repo.git(&["branch", "-q", "--", "dash-branch"]);
    repo.git(&["update-ref", "refs/heads/-d", &sha]);
    for op in ["-d", "--path"] {
        let out = repo.complexity(op);
        // `-d` розв'язується як гілка, `--path` — ні, але це помилка Git, а не аргументів.
        if op == "-d" {
            assert_eq!(out.status.code(), Some(0), "{op}");
            assert_eq!(json_of(&out)["sha"], sha.as_str());
        } else {
            assert_eq!(out.status.code(), Some(2));
            assert!(!String::from_utf8(out.stderr).unwrap().contains("osavul --help"));
        }
    }
    // Іменем файла операнд не стає локальним входом.
    repo.write("HEAD", b"function h() {}\n");
    let out = repo.complexity("HEAD");
    assert_eq!(json_of(&out)["sha"], sha.as_str());
}

#[test]
fn per_file_failures_keep_other_files_and_are_reproducible() {
    let repo = Repo::new("failures");
    repo.write("a_zero.ts", b"export const z = 1;\n");
    repo.write("b_syntax.ts", b"function (\n");
    repo.write("c_utf8.ts", &[0xff, 0xfe, b'x']);
    repo.write("d_ok.ts", b"function f() { return 1; }\n");
    let sha = repo.commit("mixed");
    let first = repo.complexity(&sha);
    let second = repo.complexity(&sha);
    for out in [&first, &second] {
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stderr.is_empty());
    }
    assert_eq!(first.stdout, second.stdout);
    let v = json_of(&first);
    assert_eq!(v["complete"], false);
    assert!(v["total"].is_null());
    let files = v["files"].as_array().unwrap();
    assert_eq!(files[0]["cyclomatic"], 0);
    assert!(files[0].get("error").is_none());
    assert!(files[1]["cyclomatic"].is_null());
    assert_eq!(files[1]["error"], "syntax-error");
    assert!(files[2]["cyclomatic"].is_null());
    assert_ne!(files[2]["error"], files[1]["error"]);
    assert_eq!(files[3]["cyclomatic"], 1);
}

#[test]
fn unreadable_blob_keeps_path_and_later_objects_are_still_scored() {
    let repo = Repo::new("blob");
    repo.write("a.ts", b"function a() { return 1; }\n");
    repo.write("b.ts", b"function b() { return 2; }\n");
    repo.write("c.ts", b"function c() { if (1) { return 3; } }\n");
    let sha = repo.commit("three");
    let oid = repo.git(&["rev-parse", "HEAD:b.ts"]);
    let obj = repo.loose_object(&oid);
    let mut perm = fs::metadata(&obj).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o644);
    fs::set_permissions(&obj, perm).unwrap();
    fs::write(&obj, b"not a zlib stream").unwrap();

    let first = repo.complexity(&sha);
    let second = repo.complexity(&sha);
    for out in [&first, &second] {
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stderr.is_empty());
    }
    assert_eq!(first.stdout, second.stdout);
    let v = json_of(&first);
    assert_eq!(v["complete"], false);
    assert!(v["total"].is_null());
    let files = v["files"].as_array().unwrap();
    assert_eq!(files[0]["cyclomatic"], 1);
    assert_eq!(files[1]["path"], "b.ts");
    assert!(files[1]["cyclomatic"].is_null());
    assert_eq!(files[1]["error"], "read-error");
    assert_eq!(files[2]["cyclomatic"], 2);
}

#[test]
fn unreadable_tree_is_fatal() {
    let repo = Repo::new("tree");
    repo.write("a.ts", b"function a() {}\n");
    let sha = repo.commit("one");
    let tree = repo.git(&["rev-parse", "HEAD^{tree}"]);
    let obj = repo.loose_object(&tree);
    let mut perm = fs::metadata(&obj).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o644);
    fs::set_permissions(&obj, perm).unwrap();
    fs::remove_file(&obj).unwrap();
    let out = repo.complexity(&sha);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
}

#[test]
fn unknown_revision_outside_repository_and_missing_git_are_fatal() {
    let repo = Repo::new("fatal");
    repo.write("a.ts", b"function a() {}\n");
    repo.commit("one");
    let unknown = repo.complexity("no-such-rev");
    repo.write("b.ts", b"function b() {}\n");
    let second = repo.commit("first release");
    repo.git(&["tag", "release", "HEAD~1"]);
    repo.git(&["branch", "release", "HEAD"]);
    let ambiguous = repo.complexity("release");
    // Вираз пошуку за повідомленням Git розв'язує однозначно.
    let by_message = repo.complexity(":/first release");
    assert_eq!(by_message.status.code(), Some(0));
    assert_eq!(json_of(&by_message)["sha"], second.as_str());
    assert_eq!(json_of(&repo.complexity("refs/heads/release"))["sha"], second.as_str());
    assert_eq!(json_of(&repo.complexity("refs/tags/release"))["sha"], repo.git(&["rev-parse", "HEAD~1"]).as_str());
    let outside = {
        let dir = repo.home.join("outside");
        fs::create_dir_all(&dir).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
        cmd.args(["complexity", "HEAD"]).current_dir(&dir);
        isolate(&mut cmd, &repo.home);
        cmd.env("GIT_CEILING_DIRECTORIES", &repo.home);
        cmd.output().unwrap()
    };
    let no_git = {
        let empty = repo.home.join("empty-path");
        fs::create_dir_all(&empty).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
        cmd.args(["complexity", "HEAD"]).current_dir(&repo.root).env("PATH", &empty);
        cmd.output().unwrap()
    };
    for out in [&unknown, &ambiguous, &outside, &no_git] {
        assert_eq!(out.status.code(), Some(2));
        assert!(out.stdout.is_empty());
        assert!(!out.stderr.is_empty());
    }
}

#[test]
fn partial_clone_does_not_fetch_missing_blobs() {
    let source = Repo::new("partial-src");
    source.write("a.ts", b"function a() {}\n");
    source.commit("one");
    source.git(&["config", "uploadpack.allowFilter", "true"]);
    let clone = source.home.join("clone");
    let mut cmd = Command::new("git");
    cmd.args(["clone", "-q", "--filter=blob:none", "--no-checkout"])
        .arg(format!("file://{}", source.root.display()))
        .arg(&clone);
    isolate(&mut cmd, &source.home);
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let out = source.run(&clone, &["complexity", "HEAD"]);
    assert_eq!(out.status.code(), Some(1));
    let v = json_of(&out);
    assert_eq!(v["files"][0]["error"], "read-error");
    assert!(v["total"].is_null());
    let mut cmd = Command::new("git");
    cmd.args(["rev-list", "--objects", "--missing=print", "HEAD"]).current_dir(&clone);
    isolate(&mut cmd, &source.home);
    cmd.env("GIT_NO_LAZY_FETCH", "1");
    let listing = String::from_utf8(cmd.output().unwrap().stdout).unwrap();
    assert!(listing.lines().any(|l| l.starts_with('?')), "blob must stay missing: {listing}");
}

#[test]
fn stdout_write_failure_is_fatal() {
    let repo = Repo::new("full");
    repo.write("a.ts", b"function a() {}\n");
    repo.commit("one");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
    cmd.args(["complexity", "HEAD"])
        .current_dir(&repo.root)
        .stdout(fs::OpenOptions::new().write(true).open("/dev/full").unwrap());
    isolate(&mut cmd, &repo.home);
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(!out.stderr.is_empty());
}

fn local_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("loc-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    set_mode(&dir, 0o755);
    dir
}

fn set_mode(path: &Path, mode: u32) {
    std::os::unix::fs::PermissionsExt::set_mode(&mut fs::metadata(path).unwrap().permissions(), mode);
    fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode)).unwrap();
}

/// Запуск без Git у PATH; `restricted` — непривілейований користувач, якщо тест іде від root.
fn local_run(dir: &Path, args: &[&str], restricted: bool) -> Output {
    let empty = dir.parent().unwrap().join("empty-path");
    fs::create_dir_all(&empty).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
    cmd.args(args).current_dir(dir).env("PATH", &empty);
    if restricted && is_root() {
        std::os::unix::process::CommandExt::uid(&mut cmd, 65534);
        std::os::unix::process::CommandExt::gid(&mut cmd, 65534);
    }
    cmd.output().expect("не вдалося запустити osavul")
}

fn is_root() -> bool {
    let out = Command::new("id").arg("-u").output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim() == "0"
}

fn copy_corpus(dir: &Path) {
    for f in ["straight.ts", "branch.ts"] {
        fs::copy(format!("{FIXTURES}/{f}"), dir.join(f)).unwrap();
    }
}

#[test]
fn local_corpus_works_outside_git_for_files_and_directory() {
    let dir = local_dir("corpus");
    copy_corpus(&dir);
    for (arg, input, expected) in [
        ("straight.ts", "file", vec![("straight.ts", 1)]),
        ("branch.ts", "file", vec![("branch.ts", 2)]),
        (".", "directory", vec![("branch.ts", 2), ("straight.ts", 1)]),
    ] {
        let first = local_run(&dir, &["complexity", "--path", arg], false);
        let second = local_run(&dir, &["complexity", "--path", arg], false);
        assert_eq!(first.status.code(), Some(0), "{arg}");
        assert!(first.stderr.is_empty());
        assert_eq!(first.stdout, second.stdout);
        let v = json_of(&first);
        assert!(v.as_object().unwrap().contains_key("sha"));
        assert!(v["sha"].is_null());
        assert_eq!(v["scope"]["input"], input);
        assert_eq!(v["complete"], true);
        let expected: Vec<(String, serde_json::Value)> =
            expected.into_iter().map(|(p, n)| (p.to_string(), serde_json::json!(n))).collect();
        assert_eq!(pairs(&v), expected);
        assert_eq!(v["total"], expected.iter().map(|e| e.1.as_u64().unwrap()).sum::<u64>());
    }
}

#[test]
fn local_empty_selection_is_complete_zero() {
    let dir = local_dir("empty");
    fs::write(dir.join("a.tsx"), b"x").unwrap();
    let out = local_run(&dir, &["complexity", "--path", "."], false);
    assert_eq!(out.status.code(), Some(0));
    let v = json_of(&out);
    assert_eq!((v["files"].as_array().unwrap().len(), v["complete"].clone(), v["total"].clone()), (0, true.into(), 0.into()));
}

#[test]
fn local_non_utf8_name_is_reported_as_path_bytes() {
    use std::os::unix::ffi::OsStrExt;
    let dir = local_dir("bytes");
    fs::write(dir.join(std::ffi::OsStr::from_bytes(b"\xff.ts")), b"function f() {}\n").unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
    cmd.args(["complexity", "--path"]).arg(&dir);
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v = json_of(&out);
    assert!(v["files"][0]["path"].is_null());
    assert_eq!(v["files"][0]["pathBytes"], "ff2e7473");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
    cmd.args(["complexity", "--path"]).arg(dir.join(std::ffi::OsStr::from_bytes(b"\xff.ts")));
    let single = cmd.output().unwrap();
    assert_eq!(single.status.code(), Some(0));
    assert_eq!(json_of(&single)["files"][0]["pathBytes"], "ff2e7473");
}

#[test]
fn local_mixed_failures_report_each_reason_and_keep_later_files() {
    let dir = local_dir("mixed");
    fs::write(dir.join("a_zero.ts"), b"export const z = 1;\n").unwrap();
    fs::write(dir.join("b_syntax.ts"), b"function (\n").unwrap();
    fs::write(dir.join("c_utf8.ts"), [0xff, 0xfe, b'x']).unwrap();
    fs::write(dir.join("d_ok.ts"), b"function f() { return 1; }\n").unwrap();
    let first = local_run(&dir, &["complexity", "--path", "."], false);
    let second = local_run(&dir, &["complexity", "--path", "."], false);
    assert_eq!(first.status.code(), Some(1));
    assert!(first.stderr.is_empty());
    assert_eq!(first.stdout, second.stdout);
    let v = json_of(&first);
    assert_eq!(v["complete"], false);
    assert!(v["total"].is_null());
    let got: Vec<_> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["path"].as_str().unwrap().to_string(), f["cyclomatic"].clone(), f.get("error").cloned()))
        .collect();
    let null = serde_json::Value::Null;
    assert_eq!(
        got,
        vec![
            ("a_zero.ts".to_string(), 0.into(), None),
            ("b_syntax.ts".to_string(), null.clone(), Some("syntax-error".into())),
            ("c_utf8.ts".to_string(), null, Some("invalid-utf8".into())),
            ("d_ok.ts".to_string(), 1.into(), None),
        ]
    );
}

fn assert_fatal(out: &Output) {
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
}

#[test]
fn local_fatal_inputs_have_empty_stdout() {
    let dir = local_dir("fatal");
    fs::write(dir.join("a.txt"), b"x").unwrap();
    assert_fatal(&local_run(&dir, &["complexity", "--path", "missing.ts"], false));
    assert_fatal(&local_run(&dir, &["complexity", "--path", "a.txt"], false));
}

#[test]
fn local_unreadable_file_keeps_record_and_scores_the_next_file() {
    let dir = local_dir("unreadable");
    fs::write(dir.join("a.ts"), b"function a() {}\n").unwrap();
    fs::write(dir.join("b.ts"), b"function b() {}\n").unwrap();
    fs::write(dir.join("c.ts"), b"function c() { if (1) {} }\n").unwrap();
    for f in ["a.ts", "b.ts", "c.ts"] {
        set_mode(&dir.join(f), 0o644);
    }
    let control = local_run(&dir, &["complexity", "--path", "."], true);
    assert_eq!(control.status.code(), Some(0), "{}", String::from_utf8_lossy(&control.stderr));
    set_mode(&dir.join("b.ts"), 0o000);
    let out = local_run(&dir, &["complexity", "--path", "."], true);
    assert_eq!(out.status.code(), Some(1));
    let v = json_of(&out);
    assert_eq!(v["files"][0]["cyclomatic"], 1);
    assert_eq!(v["files"][1]["path"], "b.ts");
    assert!(v["files"][1]["cyclomatic"].is_null());
    assert_eq!(v["files"][1]["error"], "read-error");
    assert_eq!(v["files"][2]["cyclomatic"], 2);
    assert!(v["total"].is_null());
    assert_eq!(v["complete"], false);
}

#[test]
fn local_unlistable_nested_directory_is_fatal_even_after_good_files() {
    let dir = local_dir("unlistable");
    fs::write(dir.join("a.ts"), b"function a() {}\n").unwrap();
    set_mode(&dir.join("a.ts"), 0o644);
    let sub = dir.join("sub");
    fs::create_dir(&sub).unwrap();
    set_mode(&sub, 0o755);
    fs::write(sub.join("b.ts"), b"function b() {}\n").unwrap();
    set_mode(&sub.join("b.ts"), 0o644);
    let control = local_run(&dir, &["complexity", "--path", "."], true);
    assert_eq!(control.status.code(), Some(0), "{}", String::from_utf8_lossy(&control.stderr));
    set_mode(&sub, 0o000);
    let out = local_run(&dir, &["complexity", "--path", "."], true);
    set_mode(&sub, 0o755);
    assert_fatal(&out);
}
