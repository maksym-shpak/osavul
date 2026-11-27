//! Інтеграційні тести командного інтерфейсу: запускають зібраний бінарник і читають лише код
//! виходу, `stdout` і `stderr`. Охороняють інваріанти «Знахідки»
//! (`docs/domain/entities/finding.md`, «Інваріанти») та звітів складності, покриття й дублювання
//! (`complexity-report.md`, `coverage-report.md`, `duplication-report.md`: «Стани й інваріанти»;
//! для покриття ще «Межа перевірки»). Гарантії: код 0 — повний звіт чи відсутність знахідок, 1 —
//! неповний звіт чи знахідки, 2 — помилка аргументів, джерела чи Git з порожнім `stdout` і
//! діагностикою в `stderr`; байтова повторюваність виводу; збереження результатів решти файлів при
//! збої одного.

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
    assert!(text.contains("check <файл-або-тека>"));
    assert!(text.contains("osavul complexity <ревізія>"));
    assert!(text.contains("osavul complexity --path <файл-або-тека>"));
    assert!(text.contains("Git не потрібен"));
    assert!(text.matches("--metric <cyclomatic|cognitive|loc>").count() == 2);
    assert!(text.contains("Коди завершення complexity"));
    assert!(text.contains("osavul duplication <ревізія>"));
    assert!(text.contains("osavul duplication --path <файл-або-тека>"));
    assert!(text.contains("Коди завершення duplication"));
    assert!(text.contains("osavul coverage <ревізія> --lcov <файл>"));
    assert!(text.contains("osavul coverage --path <тека> --lcov <файл>"));
    assert!(text.contains("Коди завершення coverage"));
    assert!(text.contains("osavul gate <база> <ревізія>"));
    assert!(text.contains("Коди завершення gate"));
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

// Виявляє прийняття зайвих чи неправильно записаних аргументів замість коду 2.
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
fn gate_argument_forms_are_validated() {
    assert_usage_error(&["gate"]);
    assert_usage_error(&["gate", "HEAD"]);
    assert_usage_error(&["gate", "a", "b", "c"]);
    assert_usage_error(&["gate", "HEAD", "--path"]);
    let bad = invalid_text_arg("", "");
    let out = Command::new(env!("CARGO_BIN_EXE_osavul"))
        .args(["gate", "HEAD"])
        .arg(bad)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

#[test]
fn complexity_without_revision_is_usage_error() {
    assert_usage_error(&["complexity"]);
}

#[test]
fn duplication_argument_forms_are_validated() {
    assert_usage_error(&["duplication"]);
    assert_usage_error(&["duplication", "HEAD", "--metric", "loc"]);
    assert_usage_error(&["duplication", "--path", "a.ts", "extra"]);
    assert_usage_error(&["duplication", "--path=a.ts", "a.ts"]);
    assert_usage_error(&["duplication", "a.ts", "--path"]);
    let bad = invalid_text_arg("", "");
    let out = Command::new(env!("CARGO_BIN_EXE_osavul"))
        .arg("duplication")
        .arg(bad)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

/// Текстовий аргумент, який не подається в UTF-8: невалідний байт на Unix, непарний сурогат на Windows.
/// Файл на диску для нього не потрібен.
#[cfg(unix)]
fn invalid_text_arg(prefix: &str, suffix: &str) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    let mut bytes = prefix.as_bytes().to_vec();
    bytes.push(0xff);
    bytes.extend_from_slice(suffix.as_bytes());
    std::ffi::OsString::from_vec(bytes)
}

#[cfg(windows)]
fn invalid_text_arg(prefix: &str, suffix: &str) -> std::ffi::OsString {
    use std::os::windows::ffi::OsStringExt;
    let mut units: Vec<u16> = prefix.encode_utf16().collect();
    units.push(0xD800);
    units.extend(suffix.encode_utf16());
    std::ffi::OsString::from_wide(&units)
}

fn temp_file(name: &str, content: &[u8]) -> PathBuf {
    let dir =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("check-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, content).unwrap();
    path
}

/// Повертає (код, stdout) і перевіряє порожній stderr для кодів 0 та 1.
const MSG: &str = "тест порожній: напишіть перевірку в тілі тесту або видаліть його";

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

// Виявляє хибні спрацювання чи пропуски правила на корпусі та відмінність двох запусків.
#[test]
fn corpus_matches_expected_tsv() {
    let dir = "tests/fixtures/empty-test";
    let tsv = fs::read_to_string(format!("{dir}/expected.tsv")).unwrap();
    let mut rows = tsv.lines();
    assert_eq!(rows.next(), Some("path\tpair\tlines"));
    let mut count = 0;
    for row in rows {
        let f: Vec<&str> = row.split('\t').collect();
        let path = format!("{dir}/{}", f[0]);
        let (code, expected) = match f[2] {
            "[]" => (0, String::new()),
            l => (
                1,
                format!("{path}:{}: empty-test: {MSG}\n", &l[1..l.len() - 1]),
            ),
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
    assert_eq!(count, 10);
    // Явний файл з чужим суфіксом — помилка, навіть якщо вміст є правильним Rust.
    let foreign = temp_file("foreign.txt", b"#[test]\nfn a() {}\n");
    assert_check_error(&["check", foreign.to_str().unwrap()]);
}

// Виявляє зміну порядку знахідок або номера рядка `fn` при CRLF і багаторядковому атрибуті.
#[test]
fn two_findings_in_order_with_crlf_and_multiline_attribute() {
    let src = "// c\r\n#[test]\r\nfn a() {}\r\n\r\n#[\r\n  test\r\n]\r\n/* x\r\n*/ pub(crate)\r\nfn b() { /* a /* nested */ b */ // t\r\n}\r\n";
    let path = temp_file("any_name.rs", src.as_bytes());
    // Робочий каталог — тимчасовий, тож тест не залежить від розміщення CARGO_TARGET_DIR.
    let p = format!("./{}", path.file_name().unwrap().to_str().unwrap());
    let out = run_in(path.parent(), &["check", &p]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        out.stdout,
        format!("{p}:3: empty-test: {MSG}\n{p}:10: empty-test: {MSG}\n").as_bytes()
    );
    assert!(out.stderr.is_empty());
}

// Виявляє зсув номерів рядків через BOM або shebang.
#[test]
fn bom_and_shebang_keep_physical_lines() {
    let (code, out) = check_src("bom.rs", "\u{feff}#!/usr/bin/env x\n#[test]\nfn a() {}\n");
    assert_eq!(
        (code, out.as_str()),
        (
            1,
            "P:3: empty-test: тест порожній: напишіть перевірку в тілі тесту або видаліть його\n"
        )
    );
}

#[test]
fn embedded_modules_count_but_other_items_do_not() {
    let src = "mod m {\n    #[test] pub(crate) fn a() {}\n}\n#[cfg(test)]\nmod t {\n    #[test]\n    fn b() {}\n}\n\
        #[test] async fn c() {}\n#[test] const fn d() {}\n#[test] unsafe fn e() {}\n#[test] extern \"C\" fn f() {}\n\
        struct S;\nimpl S { #[test] fn g() {} }\nfn h() { #[test] fn i() {} }\n";
    let (code, out) = check_src("mods.rs", src);
    assert_eq!(
        (code, out.as_str()),
        (
            1,
            "P:2: empty-test: тест порожній: напишіть перевірку в тілі тесту або видаліть його\nP:7: empty-test: тест порожній: напишіть перевірку в тілі тесту або видаліть його\n"
        )
    );
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
        (
            "#[test]\nfn a() -> as_type! { () } {}",
            (
                1,
                "P:2: empty-test: тест порожній: напишіть перевірку в тілі тесту або видаліть його\n",
            ),
        ),
        (
            "#[test]\nfn a() -> ! {}",
            (
                1,
                "P:2: empty-test: тест порожній: напишіть перевірку в тілі тесту або видаліть його\n",
            ),
        ),
        (
            "mod परीक्षा {\n#[test]\nfn a() {}\n}",
            (
                1,
                "P:3: empty-test: тест порожній: напишіть перевірку в тілі тесту або видаліть його\n",
            ),
        ),
        (
            "#!\n[allow(dead_code)]\n#[test]\nfn a() {}\n",
            (
                1,
                "P:4: empty-test: тест порожній: напишіть перевірку в тілі тесту або видаліть його\n",
            ),
        ),
        (
            "#! /* c */\n// d\n[allow(dead_code)]\n#[test]\nfn a() {}\n",
            (
                1,
                "P:5: empty-test: тест порожній: напишіть перевірку в тілі тесту або видаліть його\n",
            ),
        ),
    ];
    for (src, (code, out)) in cases {
        assert_eq!(check_src("sig.rs", src), (code, out.to_string()), "{src}");
    }
}

// Виявляє часткову відповідь: помилка розбору після знахідки має давати код 2 без `stdout`.
#[test]
fn late_expression_error_leaves_no_partial_report() {
    let path = temp_file(
        "late.rs",
        b"#[test]\nfn a() {}\nfn b() { let x = (1 +); }\n",
    );
    assert_check_error(&["check", path.to_str().unwrap()]);
}

#[test]
fn external_module_and_unexpanded_macro_need_nothing_else() {
    let src =
        "mod missing;\ncompile_error!(\"no\");\nfn typed() -> u8 { \"x\" }\n#[test]\nfn a() {}\n";
    assert_eq!(
        check_src("ext.rs", src),
        (
            1,
            "P:5: empty-test: тест порожній: напишіть перевірку в тілі тесту або видаліть його\n"
                .into()
        )
    );
}

// Виявляє код виходу, відмінний від 2, коли файл відсутній або не є UTF-8.
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
    // Порожній файл замість `/dev/null`, якого на Windows немає.
    let empty_config = home.join("empty-gitconfig");
    fs::write(&empty_config, b"").unwrap();
    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_GLOBAL", &empty_config)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
}

/// Тека, з якої справний Git не бачить репозиторію: пошук обмежено її батьком, а успадковані
/// `GIT_*` очищено. Без цього `target/` усередині робочого дерева Осавула підміняє «поза репозиторієм».
fn outside_dir(parent: &Path, home: &Path, name: &str) -> PathBuf {
    let dir = parent.join(name);
    fs::create_dir_all(&dir).unwrap();
    let mut cmd = outside_command("git", &dir, home);
    cmd.args(["rev-parse", "--git-dir"])
        .env("LC_ALL", "C")
        .env("LANG", "C");
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("не вдалося запустити git у {}: {e}", dir.display()));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && stderr.contains("not a git repository"),
        "передумова порушена: у {} виявлено репозиторій або інша помилка Git: {:?}, stdout {:?}, stderr {stderr}",
        dir.display(),
        out.status,
        String::from_utf8_lossy(&out.stdout)
    );
    dir
}

/// Команда для теки з [`outside_dir`] із тими самими умовами пошуку Git.
fn outside_command(program: impl AsRef<std::ffi::OsStr>, dir: &Path, home: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(dir);
    isolate(&mut cmd, home);
    cmd.env("GIT_CEILING_DIRECTORIES", dir.parent().unwrap());
    cmd
}

struct Repo {
    root: PathBuf,
    home: PathBuf,
}

impl Repo {
    fn new(name: &str) -> Repo {
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("cx-{}-{name}", std::process::id()));
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
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    /// Запускає Git із байтами в stdin (для шляхів, які не є UTF-8).
    fn git_stdin(&self, args: &[&str], input: &[u8]) -> String {
        use std::io::Write;
        use std::process::Stdio;
        let mut cmd = Command::new("git");
        cmd.args(args)
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        isolate(&mut cmd, &self.home);
        let mut child = cmd.spawn().unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
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
        self.root
            .join(".git/objects")
            .join(&oid[..2])
            .join(&oid[2..])
    }
}

/// Знімає захист запису з loose-об'єкта Git, щоб його можна було змінити чи вилучити.
#[cfg(unix)]
fn make_writable(path: &Path) {
    set_mode(path, 0o644);
}

#[cfg(not(unix))]
fn make_writable(path: &Path) {
    let mut perm = fs::metadata(path).unwrap().permissions();
    // На Windows атрибут readonly єдиний засіб захисту запису; Unix-семантики світового запису тут немає.
    #[allow(clippy::permissions_set_readonly_false)]
    perm.set_readonly(false);
    fs::set_permissions(path, perm).unwrap();
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
        .map(|f| {
            (
                f["path"].as_str().unwrap().to_string(),
                f["cyclomatic"].clone(),
            )
        })
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
        expected.push((
            f[0].to_string(),
            serde_json::json!(f[2].parse::<u64>().unwrap()),
        ));
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
    // `add -A` стоїть першим: він вилучив би з індексу записи без файла в робочій теці.
    repo.git(&["add", "-A"]);
    let target = repo.home.join("link-target");
    fs::write(&target, b"a.ts").unwrap();
    let link_blob = repo.git(&["hash-object", "-w", target.to_str().unwrap()]);
    repo.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("120000,{link_blob},link.ts"),
    ]);
    repo.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        "160000,1111111111111111111111111111111111111111,sub.ts",
    ]);
    repo.git(&["update-index", "--chmod=+x", "run.ts"]);
    repo.git(&["commit", "-q", "-m", "one"]);
    let old = repo.git(&["rev-parse", "HEAD"]);
    let modes = repo.git(&["ls-tree", &old, "a.ts", "run.ts", "link.ts", "sub.ts"]);
    let modes: Vec<&str> = modes
        .lines()
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(modes, ["100644", "120000", "100755", "160000"]);
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
    let expected: Vec<(String, serde_json::Value)> = expected
        .into_iter()
        .map(|(p, n)| (p.to_string(), serde_json::json!(n)))
        .collect();
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
fn git_tree_non_utf8_name_is_reported_as_path_bytes_without_checkout() {
    let repo = Repo::new("tree-bytes");
    let blob = repo.git_stdin(&["hash-object", "-w", "--stdin"], b"function f() {}\n");
    // Ім'я йде в індекс байтами, без файла в робочій теці й без перекодування.
    let mut info = format!("100644 {blob}\t").into_bytes();
    info.extend_from_slice(b"d\xffir/\xff.ts\n");
    repo.git_stdin(&["update-index", "--add", "--index-info"], &info);
    repo.git(&["commit", "-q", "-m", "bytes"]);
    let out = repo.complexity("HEAD");
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
    let v = json_of(&out);
    assert_eq!(v["files"].as_array().unwrap().len(), 1);
    assert!(v["files"][0]["path"].is_null());
    assert_eq!(
        v["files"][0]["pathBytes"],
        "64ff6972 2fff2e7473".replace(' ', "")
    );
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
            assert!(
                !String::from_utf8(out.stderr)
                    .unwrap()
                    .contains("osavul --help")
            );
        }
    }
    // Іменем файла операнд не стає локальним входом.
    repo.write("HEAD", b"function h() {}\n");
    let out = repo.complexity("HEAD");
    assert_eq!(json_of(&out)["sha"], sha.as_str());
}

// Виявляє втрату успішних файлів поруч зі збійними, код виходу не 1 або різні байти двох запусків.
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
    make_writable(&obj);
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
    make_writable(&obj);
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
    assert_eq!(
        json_of(&repo.complexity("refs/heads/release"))["sha"],
        second.as_str()
    );
    assert_eq!(
        json_of(&repo.complexity("refs/tags/release"))["sha"],
        repo.git(&["rev-parse", "HEAD~1"]).as_str()
    );
    let outside = {
        let dir = outside_dir(&repo.home, &repo.home, "outside");
        let mut cmd = outside_command(env!("CARGO_BIN_EXE_osavul"), &dir, &repo.home);
        cmd.args(["complexity", "HEAD"]);
        cmd.output().unwrap()
    };
    let no_git = {
        let empty = repo.home.join("empty-path");
        fs::create_dir_all(&empty).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
        cmd.args(["complexity", "HEAD"])
            .current_dir(&repo.root)
            .env("PATH", &empty);
        cmd.output().unwrap()
    };
    for out in [&unknown, &ambiguous, &outside, &no_git] {
        assert_eq!(out.status.code(), Some(2));
        assert!(out.stdout.is_empty());
        assert!(!out.stderr.is_empty());
    }
}

/// URL локального транспорту: лише `file://` змушує Git справді фільтрувати об'єкти.
fn file_url(path: &Path) -> String {
    let p = path.display().to_string();
    // Роздільник замінюємо лише на Windows: у POSIX `\` є частиною імені.
    #[cfg(windows)]
    let p = p.replace('\\', "/");
    if p.starts_with('/') {
        format!("file://{p}")
    } else {
        format!("file:///{p}")
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
        .arg(file_url(&source.root))
        .arg(&clone);
    isolate(&mut cmd, &source.home);
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = source.run(&clone, &["complexity", "HEAD"]);
    assert_eq!(out.status.code(), Some(1));
    let v = json_of(&out);
    assert_eq!(v["files"][0]["error"], "read-error");
    assert!(v["total"].is_null());
    let mut cmd = Command::new("git");
    cmd.args(["rev-list", "--objects", "--missing=print", "HEAD"])
        .current_dir(&clone);
    isolate(&mut cmd, &source.home);
    cmd.env("GIT_NO_LAZY_FETCH", "1");
    let listing = String::from_utf8(cmd.output().unwrap().stdout).unwrap();
    assert!(
        listing.lines().any(|l| l.starts_with('?')),
        "blob must stay missing: {listing}"
    );
}

#[test]
fn stdout_write_failure_is_fatal() {
    let repo = Repo::new("full");
    repo.write("a.ts", b"function a() {}\n");
    repo.commit("one");
    let run = || {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
        cmd.args(["complexity", "HEAD"]).current_dir(&repo.root);
        isolate(&mut cmd, &repo.home);
        cmd
    };
    // Контроль: справний вхід дає непорожній звіт, тож відмова нижче стосується лише запису.
    let ok = run().output().unwrap();
    assert_eq!(ok.status.code(), Some(0));
    assert!(!ok.stdout.is_empty());

    // Читальний кінець закрито до старту процесу: запис у stdout завершується помилкою.
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut cmd = run();
    cmd.stdout(writer);
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let reason = stderr
        .trim_end()
        .strip_prefix("osavul: не вдалося записати звіт: ")
        .unwrap_or_else(|| panic!("неочікувана діагностика: {stderr}"));
    assert!(!reason.is_empty(), "порожня причина: {stderr}");
}

fn local_dir(name: &str) -> PathBuf {
    // Від root дочірній процес з UID 65534 має пройти шлях до теки, а приватний
    // каталог checkout (0700) цього не дозволяє, тож фікстури лежать у /tmp.
    #[cfg(unix)]
    let root = is_root();
    #[cfg(not(unix))]
    let root = false;
    let base = if root {
        PathBuf::from("/tmp").join(format!("osavul-local-{}", std::process::id()))
    } else {
        PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
    };
    fs::create_dir_all(&base).unwrap();
    #[cfg(unix)]
    if root {
        set_mode(&base, 0o755);
    }
    let dir = base.join(format!("loc-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    #[cfg(unix)]
    set_mode(&dir, 0o755);
    dir
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// Забороняє читання файла, доки живе значення: на POSIX — правами 000, на Windows — утримуваним
/// дескриптором без спільного доступу. Права відновлюються й у разі паніки тесту.
struct ReadDenied {
    #[cfg(unix)]
    path: PathBuf,
    #[cfg(windows)]
    _handle: fs::File,
}

impl ReadDenied {
    #[cfg(unix)]
    fn new(path: &Path) -> Self {
        set_mode(path, 0o000);
        ReadDenied {
            path: path.to_path_buf(),
        }
    }

    #[cfg(windows)]
    fn new(path: &Path) -> Self {
        use std::os::windows::fs::OpenOptionsExt;
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .unwrap();
        ReadDenied { _handle: handle }
    }
}

#[cfg(unix)]
impl Drop for ReadDenied {
    fn drop(&mut self) {
        set_mode(&self.path, 0o644);
    }
}

/// Запуск без Git у PATH; `restricted` — непривілейований користувач, якщо тест іде від root
/// (інакше root обходить заборону читання). Копія бінарника унікальна для фікстури, бо спільна
/// копія під час паралельних запусків дає `ExecutableFileBusy`.
fn local_run(dir: &Path, args: &[&str], restricted: bool) -> Output {
    let empty = dir.parent().unwrap().join("empty-path");
    fs::create_dir_all(&empty).unwrap();
    let original = PathBuf::from(env!("CARGO_BIN_EXE_osavul"));
    #[cfg(unix)]
    let drop_root = restricted && is_root();
    #[cfg(not(unix))]
    let _ = restricted;
    #[cfg(unix)]
    let bin = if drop_root {
        // Бінарник теж може лежати під приватним каталогом: копіюємо в доступне місце.
        let copy = dir.parent().unwrap().join(format!(
            "osavul-bin-{}",
            dir.file_name().unwrap().to_string_lossy()
        ));
        if !copy.exists() {
            fs::copy(&original, &copy).unwrap();
            set_mode(&copy, 0o755);
        }
        set_mode(&empty, 0o755);
        copy
    } else {
        original
    };
    #[cfg(not(unix))]
    let bin = original;
    // Паралельний fork іншого тесту може на мить утримати відкритим дескриптор запису копії.
    for attempt in 0.. {
        let mut cmd = Command::new(&bin);
        cmd.args(args).current_dir(dir).env("PATH", &empty);
        #[cfg(unix)]
        if drop_root {
            std::os::unix::process::CommandExt::uid(&mut cmd, 65534);
            std::os::unix::process::CommandExt::gid(&mut cmd, 65534);
        }
        match cmd.output() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 50 => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            r => return r.expect("не вдалося запустити osavul"),
        }
    }
    unreachable!()
}

#[cfg(unix)]
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
    let root = local_dir("corpus");
    let dir = outside_dir(&root, &root, "work");
    copy_corpus(&dir);
    let empty = root.join("empty-path");
    fs::create_dir_all(&empty).unwrap();
    for (arg, input, expected) in [
        ("straight.ts", "file", vec![("straight.ts", 1)]),
        ("branch.ts", "file", vec![("branch.ts", 2)]),
        (".", "directory", vec![("branch.ts", 2), ("straight.ts", 1)]),
    ] {
        let run = || {
            let mut cmd = outside_command(env!("CARGO_BIN_EXE_osavul"), &dir, &root);
            cmd.args(["complexity", "--path", arg]).env("PATH", &empty);
            cmd.output().unwrap()
        };
        let first = run();
        let second = run();
        assert_eq!(first.status.code(), Some(0), "{arg}");
        assert!(first.stderr.is_empty());
        assert_eq!(first.stdout, second.stdout);
        let v = json_of(&first);
        assert!(v.as_object().unwrap().contains_key("sha"));
        assert!(v["sha"].is_null());
        assert_eq!(v["scope"]["input"], input);
        assert_eq!(v["complete"], true);
        let expected: Vec<(String, serde_json::Value)> = expected
            .into_iter()
            .map(|(p, n)| (p.to_string(), serde_json::json!(n)))
            .collect();
        assert_eq!(pairs(&v), expected);
        assert_eq!(
            v["total"],
            expected.iter().map(|e| e.1.as_u64().unwrap()).sum::<u64>()
        );
    }
}

#[test]
fn local_empty_selection_is_complete_zero() {
    let dir = local_dir("empty");
    fs::write(dir.join("a.tsx"), b"x").unwrap();
    let out = local_run(&dir, &["complexity", "--path", "."], false);
    assert_eq!(out.status.code(), Some(0));
    let v = json_of(&out);
    assert_eq!(
        (
            v["files"].as_array().unwrap().len(),
            v["complete"].clone(),
            v["total"].clone()
        ),
        (0, true.into(), 0.into())
    );
}

#[test]
fn local_nested_unicode_name_is_reported_exactly() {
    let dir = local_dir("unicode");
    fs::create_dir_all(dir.join("дані")).unwrap();
    fs::write(dir.join("дані").join("🌿.ts"), b"function f() {}\n").unwrap();
    let out = local_run(&dir, &["complexity", "--path", "."], false);
    assert_eq!(out.status.code(), Some(0));
    let v = json_of(&out);
    assert_eq!(v["files"][0]["path"], "дані/🌿.ts");
    assert!(v["files"][0].get("pathBytes").is_none_or(|b| b.is_null()));
}

// Фізичне ім'я з невалідним UTF-8 можна створити на Linux; APFS таких імен не допускає, а Windows
// подає імена інакше. Байти шляху з дерева Git перевіряє окремий тест без checkout.
#[cfg(target_os = "linux")]
#[test]
fn local_non_utf8_name_is_reported_as_path_bytes() {
    use std::os::unix::ffi::OsStrExt;
    let dir = local_dir("bytes");
    fs::write(
        dir.join(std::ffi::OsStr::from_bytes(b"\xff.ts")),
        b"function f() {}\n",
    )
    .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
    cmd.args(["complexity", "--path"]).arg(&dir);
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v = json_of(&out);
    assert!(v["files"][0]["path"].is_null());
    assert_eq!(v["files"][0]["pathBytes"], "ff2e7473");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
    cmd.args(["complexity", "--path"])
        .arg(dir.join(std::ffi::OsStr::from_bytes(b"\xff.ts")));
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
        .map(|f| {
            (
                f["path"].as_str().unwrap().to_string(),
                f["cyclomatic"].clone(),
                f.get("error").cloned(),
            )
        })
        .collect();
    let null = serde_json::Value::Null;
    assert_eq!(
        got,
        vec![
            ("a_zero.ts".to_string(), 0.into(), None),
            (
                "b_syntax.ts".to_string(),
                null.clone(),
                Some("syntax-error".into())
            ),
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
    assert_fatal(&local_run(
        &dir,
        &["complexity", "--path", "missing.ts"],
        false,
    ));
    assert_fatal(&local_run(&dir, &["complexity", "--path", "a.txt"], false));
}

#[test]
fn local_unreadable_file_keeps_record_and_scores_the_next_file() {
    let dir = local_dir("unreadable");
    fs::write(dir.join("a.ts"), b"function a() {}\n").unwrap();
    fs::write(dir.join("b.ts"), b"function b() {}\n").unwrap();
    fs::write(dir.join("c.ts"), b"function c() { if (1) {} }\n").unwrap();
    #[cfg(unix)]
    for f in ["a.ts", "b.ts", "c.ts"] {
        set_mode(&dir.join(f), 0o644);
    }
    let control = local_run(&dir, &["complexity", "--path", "."], true);
    assert_eq!(
        control.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&control.stderr)
    );
    let _denied = ReadDenied::new(&dir.join("b.ts"));
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

#[cfg(unix)]
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
    assert_eq!(
        control.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&control.stderr)
    );
    set_mode(&sub, 0o000);
    let out = local_run(&dir, &["complexity", "--path", "."], true);
    set_mode(&sub, 0o755);
    assert_fatal(&out);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("sub"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

const ONE_IF: &str = "export function f(x: number) { if (x) { return 1; } }\n";

#[test]
fn metric_option_errors_are_usage_errors_before_source_access() {
    for args in [
        &["complexity", "HEAD", "--metric"][..],
        &["complexity", "HEAD", "--metric", "other"],
        &["complexity", "HEAD", "--metric", "Cognitive"],
        &["complexity", "HEAD", "--metric", "Loc"],
        &["complexity", "--path", "a.ts", "--metric", "Loc"],
        &[
            "complexity",
            "HEAD",
            "--metric",
            "cognitive",
            "--metric",
            "cognitive",
        ],
        &["complexity", "HEAD", "--metric", "cognitive", "extra"],
        &["complexity", "HEAD", "--metrics", "cognitive"],
        &["complexity", "--path", "a.ts", "--metric"],
        &["complexity", "--path", "a.ts", "--metric", "other"],
        &[
            "complexity",
            "--path",
            "a.ts",
            "--metric",
            "cognitive",
            "extra",
        ],
    ] {
        assert_usage_error(args);
    }
}

#[test]
fn metric_value_outside_utf8_is_usage_error() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
    cmd.args(["complexity", "HEAD", "--metric"])
        .arg(invalid_text_arg("cog", "nitive"));
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("osavul --help")
    );
}

#[test]
fn bare_metric_operand_is_a_revision() {
    let repo = Repo::new("metric-operand");
    repo.write("a.ts", ONE_IF.as_bytes());
    let sha = repo.commit("one");
    repo.git(&["update-ref", "refs/heads/--metric", &sha]);
    let out = repo.complexity("--metric");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(json_of(&out)["sha"], sha.as_str());
}

#[test]
fn commit_metric_selects_methodology_and_score_key() {
    let repo = Repo::new("metric-commit");
    repo.write("a.ts", ONE_IF.as_bytes());
    repo.commit("one");
    let out = repo.run(&repo.root, &["complexity", "HEAD", "--metric", "cognitive"]);
    assert_eq!(out.status.code(), Some(0));
    let v = json_of(&out);
    assert_eq!(
        v["methodology"],
        serde_json::json!({"id": "ts-cognitive", "version": 1})
    );
    assert_eq!(v["files"][0]["cognitive"], 1);
    assert!(v["files"][0].get("cyclomatic").is_none());
    assert_eq!(v["total"], 1);

    let out = repo.run(&repo.root, &["complexity", "HEAD", "--metric", "loc"]);
    assert_eq!(out.status.code(), Some(0));
    let v = json_of(&out);
    assert_eq!(
        v["methodology"],
        serde_json::json!({"id": "ts-loc", "version": 1})
    );
    assert_eq!(v["files"][0]["loc"], 1);
    assert_eq!(v["total"], 1);

    let implicit = repo.complexity("HEAD");
    let explicit = repo.run(
        &repo.root,
        &["complexity", "HEAD", "--metric", "cyclomatic"],
    );
    assert_eq!(implicit.status.code(), Some(0));
    assert_eq!(implicit.status.code(), explicit.status.code());
    assert_eq!(implicit.stdout, explicit.stdout);
    let v = json_of(&implicit);
    assert_eq!(v["methodology"]["id"], "ts-cyclomatic");
    assert_eq!(v["files"][0]["cyclomatic"], 2);
}

#[test]
fn local_metric_selects_methodology_and_score_key() {
    let dir = local_dir("metric");
    fs::write(dir.join("a.ts"), ONE_IF).unwrap();
    let out = local_run(
        &dir,
        &["complexity", "--path", "a.ts", "--metric", "cognitive"],
        false,
    );
    assert_eq!(out.status.code(), Some(0));
    let v = json_of(&out);
    assert_eq!(
        v["methodology"],
        serde_json::json!({"id": "ts-cognitive", "version": 1})
    );
    assert_eq!(v["files"][0]["cognitive"], 1);
    assert!(v["files"][0].get("cyclomatic").is_none());
    assert_eq!(v["total"], 1);

    let out = local_run(
        &dir,
        &["complexity", "--path", "a.ts", "--metric", "loc"],
        false,
    );
    assert_eq!(out.status.code(), Some(0));
    let v = json_of(&out);
    assert_eq!(
        v["methodology"],
        serde_json::json!({"id": "ts-loc", "version": 1})
    );
    assert_eq!(v["files"][0]["loc"], 1);
    assert_eq!(v["total"], 1);

    let implicit = local_run(&dir, &["complexity", "--path", "a.ts"], false);
    let explicit = local_run(
        &dir,
        &["complexity", "--path", "a.ts", "--metric", "cyclomatic"],
        false,
    );
    assert_eq!(implicit.status.code(), Some(0));
    assert_eq!(implicit.status.code(), explicit.status.code());
    assert_eq!(implicit.stdout, explicit.stdout);
    assert_eq!(json_of(&implicit)["files"][0]["cyclomatic"], 2);
}

// Виявляє код виходу не 1 або втрату JSON, коли збій одного файла робить звіт неповним.
#[test]
fn cognitive_file_failure_gives_incomplete_report_with_exit_one() {
    let dir = local_dir("metric-fail");
    fs::write(dir.join("a.ts"), b"function (").unwrap();
    fs::write(dir.join("b.ts"), ONE_IF).unwrap();
    let out = local_run(
        &dir,
        &["complexity", "--path", ".", "--metric", "cognitive"],
        false,
    );
    assert_eq!(out.status.code(), Some(1));
    let v = json_of(&out);
    assert_eq!(v["complete"], false);
    assert!(v["total"].is_null());
    assert!(v["files"][0]["cognitive"].is_null());
    assert_eq!(v["files"][0]["error"], "syntax-error");
    assert_eq!(v["files"][1]["cognitive"], 1);
}

const DUP_BLOCK: &str = "function f(a: number, b: number) {\n  const x = a + b;\n  const y = a - b;\n  const z = a * b;\n  const w = a / b;\n  const v = a % b;\n  const u = x + y + z;\n  const t = w + v + u;\n  const s = t + x + y;\n  const r = s + z + w;\n  const q = r * s * t + u * v * w + x * y * z;\n  const p = q - r - s - t - u - v - w - x - y - z;\n  const o = p + q + r + s + t + u + v + w + x + y + z;\n  return r + v + u + t + o + p + q;\n}\n";

#[test]
fn duplication_reports_cross_file_group_from_commit() {
    let repo = Repo::new("dup-commit");
    repo.write("a.ts", DUP_BLOCK.as_bytes());
    repo.write("b.ts", DUP_BLOCK.as_bytes());
    let sha = repo.commit("dup");
    let out = repo.run(&repo.root, &["duplication", &sha]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
    let v = json_of(&out);
    assert_eq!(v["sha"], sha.as_str());
    assert_eq!(v["methodology"]["id"], "ts-duplication");
    assert_eq!(v["scope"]["input"], "git-tree");
    let groups = v["duplications"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    let blocks = groups[0]["blocks"].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["path"], "a.ts");
    assert_eq!(blocks[1]["path"], "b.ts");
    assert_eq!(v["total"]["duplicatedFiles"], 2);
}

#[test]
fn duplication_local_file_and_directory_without_git() {
    let dir = local_dir("dup-local");
    fs::write(dir.join("a.ts"), DUP_BLOCK).unwrap();
    fs::write(dir.join("b.ts"), DUP_BLOCK).unwrap();
    let out = local_run(&dir, &["duplication", "--path", "."], false);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
    let v = json_of(&out);
    assert!(v["sha"].is_null());
    assert_eq!(v["scope"]["input"], "directory");
    assert_eq!(v["duplications"].as_array().unwrap().len(), 1);
    let out = local_run(&dir, &["duplication", "--path", "a.ts"], false);
    assert_eq!(out.status.code(), Some(0));
    let v = json_of(&out);
    assert_eq!(v["scope"]["input"], "file");
    assert!(v["duplications"].as_array().unwrap().is_empty());
}

// Виявляє змішування кодів 1 (неповний звіт із JSON) і 2 (фатальна помилка без `stdout`).
#[test]
fn duplication_incomplete_report_exits_one_and_fatal_exits_two() {
    let dir = local_dir("dup-incomplete");
    fs::write(dir.join("a.ts"), DUP_BLOCK).unwrap();
    fs::write(dir.join("bad.ts"), "let = = ;\n").unwrap();
    let out = local_run(&dir, &["duplication", "--path", "."], false);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty());
    let v = json_of(&out);
    assert_eq!(v["complete"], false);
    assert!(v["total"].is_null());
    let out = local_run(&dir, &["duplication", "--path", "missing"], false);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
}

#[test]
fn duplication_single_operand_with_leading_dash_is_a_revision() {
    let repo = Repo::new("dup-dash");
    repo.write("a.ts", b"function a() {}\n");
    let sha = repo.commit("one");
    repo.git(&["update-ref", "refs/heads/-d", &sha]);
    let out = repo.run(&repo.root, &["duplication", "-d"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(json_of(&out)["sha"], sha.as_str());
    let out = repo.run(&repo.root, &["duplication", "--path"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(
        !String::from_utf8(out.stderr)
            .unwrap()
            .contains("osavul --help")
    );
}

const COV_SRC: &[u8] = b"function f(a: number) {\n  if (a) {\n    return 1;\n  }\n  return 2;\n}\n";

fn cov_lcov(path: &str) -> String {
    format!(
        "SF:{path}\nDA:1,1\nDA:2,1\nDA:3,0\nDA:5,1\nBRDA:2,0,0,1\nBRDA:2,0,1,-\nend_of_record\n"
    )
}

// Виявляє прийняття неповних чи неприпустимих аргументів `coverage` замість коду 2.
#[test]
fn coverage_argument_errors_are_usage_errors() {
    assert_usage_error(&["coverage"]);
    assert_usage_error(&["coverage", "HEAD"]);
    assert_usage_error(&["coverage", "--path", "."]);
    assert_usage_error(&["coverage", "--path", ".", "--lcov"]);
    assert_usage_error(&["coverage", "HEAD", "--lcov", "a", "--tests"]);
    assert_usage_error(&["coverage", "HEAD", "--lcov", "a", "--tests", "--lcov", "b"]);
    assert_usage_error(&["coverage", "HEAD", "--lcov", "a", "--bogus"]);
    assert_usage_error(&["coverage", "HEAD", "--lcov", "a", "extra"]);
    assert_usage_error(&["coverage", "--path", ".", "--lcov", "a", "--tests", "/abs"]);
    assert_usage_error(&["coverage", "--path", ".", "--lcov", "a", "--tests", "../x"]);
    assert_usage_error(&["coverage", "--path", ".", "--lcov", "a", "--tests", ""]);
    let bad = invalid_text_arg("", "");
    let out = Command::new(env!("CARGO_BIN_EXE_osavul"))
        .args(["coverage"])
        .arg(bad)
        .args(["--lcov", "a"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

#[test]
fn coverage_local_directory_without_git_and_option_forms() {
    let dir = local_dir("cov-local");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::create_dir_all(dir.join("test")).unwrap();
    fs::write(dir.join("src/a.ts"), COV_SRC).unwrap();
    fs::write(dir.join("test/t.ts"), COV_SRC).unwrap();
    fs::create_dir_all(dir.join("reports")).unwrap();
    // Відносний SF шукається від теки звіту, потім від кореня.
    fs::write(dir.join("reports/one.info"), cov_lcov("src/a.ts")).unwrap();
    fs::write(
        dir.join("reports/two.info"),
        "SF:test/t.ts\nDA:99,1\nend_of_record\n",
    )
    .unwrap();
    let args = [
        "coverage",
        "--path",
        ".",
        "--tests",
        "test",
        "--lcov",
        "reports/one.info",
        "--lcov",
        "reports/two.info",
        "--tests",
    ];
    assert_fatal(&local_run(&dir, &args, false));
    let out = local_run(&dir, &args[..9], false);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
    let v = json_of(&out);
    assert!(v["sha"].is_null());
    assert_eq!(v["scope"]["input"], "directory");
    assert_eq!(v["methodology"]["id"], "ts-coverage");
    let files = v["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "src/a.ts");
    assert_eq!(files[0]["inReport"], true);
    assert_eq!(files[0]["linesToCover"], 4);
    assert_eq!(files[0]["uncoveredLines"], 1);
    assert_eq!(files[0]["conditionsToCover"], 2);
    assert_eq!(files[0]["uncoveredConditions"], 1);
    assert_eq!(v["total"]["coverage"], 66.7);
    // Порядок опцій довільний, повтори --tests об'єднуються.
    let reordered = local_run(
        &dir,
        &[
            "coverage",
            "--path",
            ".",
            "--lcov",
            "reports/two.info",
            "--tests",
            "x",
            "--lcov",
            "reports/one.info",
            "--tests",
            "test",
        ],
        false,
    );
    assert_eq!(reordered.stdout, out.stdout);
}

#[test]
fn coverage_zero_is_complete_and_defects_are_incomplete() {
    let dir = local_dir("cov-codes");
    fs::write(dir.join("a.ts"), COV_SRC).unwrap();
    fs::write(dir.join("b.ts"), b"function (\n").unwrap();
    fs::write(dir.join("zero.info"), "SF:a.ts\nDA:1,0\nend_of_record\n").unwrap();
    let out = local_run(
        &dir,
        &[
            "coverage",
            "--path",
            ".",
            "--lcov",
            "zero.info",
            "--tests",
            "b.ts",
        ],
        false,
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
    assert_eq!(json_of(&out)["total"]["coverage"], 0.0);

    let out = local_run(
        &dir,
        &["coverage", "--path", ".", "--lcov", "zero.info"],
        false,
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty());
    let v = json_of(&out);
    assert_eq!(v["complete"], false);
    assert!(v["total"].is_null());
    assert_eq!(v["files"][1]["error"], "syntax-error");
    assert_eq!(v["files"][0]["linesToCover"], 1);

    // Відсутній LCOV після справного — фатальна помилка.
    assert_fatal(&local_run(
        &dir,
        &[
            "coverage",
            "--path",
            ".",
            "--lcov",
            "zero.info",
            "--lcov",
            "missing.info",
        ],
        false,
    ));
    assert_fatal(&local_run(
        &dir,
        &["coverage", "--path", "a.ts", "--lcov", "zero.info"],
        false,
    ));
    assert_fatal(&local_run(
        &dir,
        &["coverage", "--path", "none", "--lcov", "zero.info"],
        false,
    ));
}

#[test]
fn coverage_lcov_resolves_from_cwd_and_report_dir() {
    let dir = local_dir("cov-dirs");
    fs::create_dir_all(dir.join("pkg/reports")).unwrap();
    fs::write(dir.join("pkg/a.ts"), COV_SRC).unwrap();
    fs::write(dir.join("pkg/reports/r.info"), cov_lcov("a.ts")).unwrap();
    // Шлях LCOV — від поточної теки; SF — від теки звіту та її батьків.
    let out = local_run(
        &dir,
        &["coverage", "--path", ".", "--lcov", "pkg/reports/r.info"],
        false,
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(json_of(&out)["files"][0]["inReport"], true);
    // Корінь, що є посиланням, розіменовується. Створення посилань на Windows потребує прав,
    // тому цей підсценарій локально там не перевіряється.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("pkg", dir.join("link")).unwrap();
        let out = local_run(
            &dir,
            &["coverage", "--path", "link", "--lcov", "pkg/reports/r.info"],
            false,
        );
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(json_of(&out)["files"][0]["path"], "a.ts");
    }
}

#[test]
fn coverage_unreadable_main_file_is_incomplete_with_good_neighbours() {
    let dir = local_dir("cov-unreadable");
    fs::write(dir.join("a.ts"), COV_SRC).unwrap();
    fs::write(dir.join("b.ts"), COV_SRC).unwrap();
    fs::write(dir.join("r.info"), cov_lcov("a.ts")).unwrap();
    #[cfg(unix)]
    for f in ["a.ts", "b.ts", "r.info"] {
        set_mode(&dir.join(f), 0o644);
    }
    let args = ["coverage", "--path", ".", "--lcov", "r.info"];
    let control = local_run(&dir, &args, true);
    assert_eq!(
        control.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&control.stderr)
    );
    assert!(json_of(&control)["files"][1]["error"].is_null());
    let _denied = ReadDenied::new(&dir.join("b.ts"));
    let out = local_run(&dir, &args, true);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty());
    let v = json_of(&out);
    assert_eq!(v["files"][0]["linesToCover"], 4);
    assert_eq!(v["files"][1]["path"], "b.ts");
    assert_eq!(v["files"][1]["error"], "read-error");
}

#[test]
fn coverage_unreadable_lcov_is_fatal() {
    let dir = local_dir("cov-lcov-denied");
    fs::write(dir.join("a.ts"), COV_SRC).unwrap();
    fs::write(dir.join("ok.info"), cov_lcov("a.ts")).unwrap();
    fs::write(dir.join("no.info"), cov_lcov("a.ts")).unwrap();
    #[cfg(unix)]
    for f in ["a.ts", "ok.info", "no.info"] {
        set_mode(&dir.join(f), 0o644);
    }
    let args = [
        "coverage", "--path", ".", "--lcov", "ok.info", "--lcov", "no.info",
    ];
    let control = local_run(&dir, &args, true);
    assert_ne!(control.status.code(), Some(2));
    let _denied = ReadDenied::new(&dir.join("no.info"));
    let out = local_run(&dir, &args, true);
    assert_fatal(&out);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("LCOV") && err.contains("no.info"), "{err}");
}

#[test]
fn coverage_old_commit_from_subdirectory_with_absolute_sf() {
    let repo = Repo::new("cov-commit");
    repo.write("src/gone.ts", COV_SRC);
    repo.write("src/deep/keep.ts", COV_SRC);
    let old = repo.commit("one");
    fs::remove_file(repo.root.join("src/gone.ts")).unwrap();
    repo.write("src/deep/keep.ts", b"const changed = 1;\n");
    repo.commit("two");

    let reports = repo.home.join("outside");
    fs::create_dir_all(&reports).unwrap();
    let lcov = format!(
        "{}{}",
        cov_lcov(repo.root.join("src/gone.ts").to_str().unwrap()),
        "SF:/elsewhere/x.js\nDA:999,1\nend_of_record\n"
    );
    fs::write(reports.join("r.info"), lcov).unwrap();
    let cwd = repo.root.join("src/deep");
    let out = repo.run(
        &cwd,
        &[
            "coverage",
            &old[..10],
            "--lcov",
            reports.join("r.info").to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stderr.is_empty());
    let v = json_of(&out);
    assert_eq!(v["sha"], old.as_str());
    assert_eq!(v["scope"]["input"], "git-tree");
    assert_eq!(v["files"][0]["path"], "src/deep/keep.ts");
    assert_eq!(v["files"][0]["inReport"], false);
    assert_eq!(v["files"][1]["path"], "src/gone.ts");
    assert_eq!(v["files"][1]["inReport"], true);
    assert_eq!(v["files"][1]["linesToCover"], 4);

    let bad = repo.run(
        &cwd,
        &[
            "coverage",
            "nope",
            "--lcov",
            reports.join("r.info").to_str().unwrap(),
        ],
    );
    assert_fatal(&bad);
    let outside = outside_dir(&repo.home, &repo.home, "plain");
    let mut cmd = outside_command(env!("CARGO_BIN_EXE_osavul"), &outside, &repo.home);
    cmd.args([
        "coverage",
        "HEAD",
        "--lcov",
        reports.join("r.info").to_str().unwrap(),
    ]);
    assert_fatal(&cmd.output().unwrap());
}

#[test]
fn coverage_bare_repository_resolves_sf_against_git_dir() {
    let repo = Repo::new("cov-bare");
    repo.write("a.ts", COV_SRC);
    repo.commit("one");
    let bare = repo.home.join("bare.git");
    repo.git(&[
        "clone",
        "-q",
        "--bare",
        repo.root.to_str().unwrap(),
        bare.to_str().unwrap(),
    ]);
    let canon = fs::canonicalize(&bare).unwrap();
    let report = repo.home.join("bare.info");
    fs::write(&report, cov_lcov(canon.join("a.ts").to_str().unwrap())).unwrap();
    let out = repo.run(
        &bare,
        &["coverage", "HEAD", "--lcov", report.to_str().unwrap()],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_of(&out);
    assert_eq!(v["files"][0]["inReport"], true);
    assert_eq!(v["files"][0]["linesToCover"], 4);
}

// --- `check` на теках: інвентар, шляхи, часткові результати ---

fn write_at(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

const EMPTY: &str = "#[test]\nfn a() {}\n";

// Виявляє хибне написання шляху теки, порядок за байтами шляху та пропуск вкладених тек.
#[test]
fn directory_report_uses_argument_spelling_order_and_skips_excluded() {
    let dir = local_dir("check-dir");
    write_at(&dir, "b.rs", EMPTY);
    write_at(&dir, "a/z.rs", &format!("\n\n{EMPTY}{EMPTY}"));
    write_at(&dir, "a.rs", EMPTY);
    write_at(&dir, "target/t.rs", EMPTY);
    write_at(&dir, "x/node_modules/n.rs", EMPTY);
    write_at(&dir, "notes.txt", "not rust (");
    for arg in [".", "./"] {
        let first = run_in(Some(&dir), &["check", arg]);
        let second = run_in(Some(&dir), &["check", arg]);
        assert_eq!(first.stdout, second.stdout);
        assert_eq!(first.status.code(), Some(1));
        assert!(first.stderr.is_empty());
        let text = String::from_utf8(first.stdout).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let prefix = "./";
        assert_eq!(lines.len(), 4, "{text}");
        assert!(lines[0].starts_with(&format!("{prefix}a.rs:2: empty-test: ")));
        assert!(lines[1].starts_with(&format!("{prefix}a/z.rs:")));
        assert!(lines[3].starts_with(&format!("{prefix}b.rs:2: ")));
        assert!(text.ends_with(&format!("{MSG}\n")));
        assert!(!text.contains("target") && !text.contains("node_modules"));
    }
}

#[test]
fn directory_without_supported_files_is_clean_and_missing_path_is_error() {
    let dir = local_dir("check-empty");
    write_at(&dir, "readme.txt", "x");
    let out = run_in(Some(&dir), &["check", "."]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    assert_check_error(&["check", dir.join("missing").to_str().unwrap()]);
}

#[test]
fn broken_file_is_reported_while_neighbour_findings_are_printed() {
    let dir = local_dir("check-partial");
    write_at(
        &dir,
        "a_broken.rs",
        "#[test]\nfn a() {}\nfn b() { let x = (1 +); }\n",
    );
    write_at(&dir, "b_good.rs", EMPTY);
    let out = run_in(Some(&dir), &["check", "."]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("./b_good.rs:2: empty-test: {MSG}\n")
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("./a_broken.rs"), "{stderr}");
    assert!(!stderr.contains("b_good"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn unreadable_file_is_reported_and_neighbours_still_checked() {
    let dir = local_dir("check-denied");
    write_at(&dir, "a.rs", EMPTY);
    write_at(&dir, "b.rs", EMPTY);
    // Потрібен справжній rustc у PATH для b.rs, тож запуск не звужується до іншого користувача;
    // від root заборона читання не діє, тож тест там нічого не доводить і пропускається.
    if is_root() {
        return;
    }
    let _denied = ReadDenied::new(&dir.join("a.rs"));
    let out = run_in(Some(&dir), &["check", "."]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("./b.rs:2: empty-test: {MSG}\n")
    );
    assert!(String::from_utf8(out.stderr).unwrap().contains("./a.rs"));
}

// Для `.rs` потрібен rustc: без нього файл — помилка (код 2), а теку без `.rs` він не зачіпає.
#[test]
fn missing_rustc_fails_rust_files_only() {
    let dir = local_dir("check-norustc");
    write_at(&dir, "a.rs", EMPTY);
    let out = local_run(&dir, &["check", "a.rs"], false);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8(out.stderr).unwrap().contains("a.rs"));
}

#[cfg(unix)]
#[test]
fn directory_with_unprintable_nested_name_fails_only_that_file() {
    let dir = local_dir("check-lf");
    write_at(&dir, "a\nb.rs", EMPTY);
    write_at(&dir, "c.rs", EMPTY);
    let out = run_in(Some(&dir), &["check", "."]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("./c.rs:2: empty-test: {MSG}\n")
    );
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("\"./a\\nb.rs\""), "{err}");
    assert!(err.contains("один запис"), "{err}");
}

#[cfg(target_os = "linux")]
#[test]
fn nested_non_utf8_name_is_printed_as_raw_bytes() {
    use std::os::unix::ffi::OsStrExt;
    let dir = local_dir("check-raw");
    let name = std::ffi::OsStr::from_bytes(b"\xff.rs");
    fs::write(dir.join(name), EMPTY).unwrap();
    let out = run_in(Some(&dir), &["check", "."]);
    assert_eq!(out.status.code(), Some(1));
    let mut expected = b"./\xff.rs:2: empty-test: ".to_vec();
    expected.extend_from_slice(format!("{MSG}\n").as_bytes());
    assert_eq!(out.stdout, expected);
}

#[test]
fn check_stdout_write_failure_is_fatal() {
    let path = temp_file("pipe.rs", EMPTY.as_bytes());
    let p = path.to_str().unwrap();
    let ok = run(&["check", p]);
    assert_eq!(ok.status.code(), Some(1));
    assert!(!ok.stdout.is_empty());

    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
    cmd.args(["check", p]).stdout(writer);
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("не вдалося записати звіт"), "{stderr}");
}

// Виявляє зсув позицій чи форматів між мовами: змішана тека, обидва написання кореня, порядок,
// кратність на одному рядку й байтова повторюваність.
#[test]
fn mixed_directory_prints_both_languages_in_order() {
    let dir = local_dir("check-mixed");
    write_at(
        &dir,
        "b.ts",
        "it('a', () => {});\nit('b', () => {}); test('c', () => {});\n",
    );
    write_at(&dir, "a.rs", &format!("\n\n{EMPTY}{EMPTY}"));
    write_at(&dir, "types.d.ts", "declare function f(): void;\n");
    write_at(&dir, "target/t.ts", "it('a', () => {});\n");
    write_at(&dir, "z/node_modules/n.ts", "it('a', () => {});\n");
    write_at(&dir, "n.txt", "it('a', () => {});\n");
    let sibling = format!("../{}", dir.file_name().unwrap().to_str().unwrap());
    let sibling_prefix = format!("{sibling}/");
    for (arg, prefix) in [
        (".", "./"),
        ("./", "./"),
        (sibling.as_str(), sibling_prefix.as_str()),
    ] {
        let first = run_in(Some(&dir), &["check", arg]);
        let second = run_in(Some(&dir), &["check", arg]);
        assert_eq!(first.stdout, second.stdout);
        assert_eq!(first.status.code(), Some(1));
        assert!(first.stderr.is_empty());
        let expected = format!(
            "{prefix}a.rs:4: empty-test: {MSG}\n{prefix}a.rs:6: empty-test: {MSG}\n\
             {prefix}b.ts:1: empty-test: {MSG}\n{prefix}b.ts:2: empty-test: {MSG}\n\
             {prefix}b.ts:2: empty-test: {MSG}\n"
        );
        assert_eq!(String::from_utf8(first.stdout).unwrap(), expected, "{arg}");
    }
    let out = run_in(Some(&dir), &["check", "./b.ts"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stdout
            .starts_with(format!("./b.ts:1: empty-test: {MSG}\n").as_bytes())
    );
}

// Виявляє часткову перевірку TypeScript: пізня діагностика відкидає й раніші знахідки файла.
#[test]
fn broken_typescript_is_rejected_whole_while_neighbour_is_printed() {
    let dir = local_dir("check-ts-partial");
    write_at(&dir, "a_broken.ts", "it('a', () => {});\nlet x = ;\n");
    write_at(&dir, "b_good.ts", "it('a', () => {});\n");
    let out = local_run(&dir, &["check", "."], false);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("./b_good.ts:1: empty-test: {MSG}\n")
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("./a_broken.ts") && stderr.contains("syntax-error"),
        "{stderr}"
    );
    assert!(!stderr.contains("b_good"), "{stderr}");
    let single = run_in(Some(&dir), &["check", "a_broken.ts"]);
    assert_eq!(single.status.code(), Some(2));
    assert!(single.stdout.is_empty());
}

#[test]
fn declaration_file_is_accepted() {
    let dir = local_dir("check-dts");
    write_at(&dir, "a.d.ts", "declare function f(): void;\n");
    let out = run_in(Some(&dir), &["check", "a.d.ts"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn unreadable_typescript_keeps_neighbour_result() {
    if is_root() {
        return;
    }
    let dir = local_dir("check-ts-denied");
    write_at(&dir, "a.ts", "it('a', () => {});\n");
    write_at(&dir, "b.ts", "it('a', () => {});\n");
    let _denied = ReadDenied::new(&dir.join("a.ts"));
    let out = local_run(&dir, &["check", "."], false);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("./b.ts:1: empty-test: {MSG}\n")
    );
    assert!(String::from_utf8(out.stderr).unwrap().contains("./a.ts"));
}

// TypeScript не потребує rustc (порожній PATH, без Git); щойно відібрано `.rs` — помилка файла.
#[test]
fn rustc_is_needed_only_when_rust_files_are_selected() {
    let dir = local_dir("check-ts-norustc");
    write_at(&dir, "a.ts", "it('a', () => {});\n");
    let ts_only = local_run(&dir, &["check", "."], false);
    assert_eq!(ts_only.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(ts_only.stdout.clone()).unwrap(),
        format!("./a.ts:1: empty-test: {MSG}\n")
    );
    assert!(ts_only.stderr.is_empty());
    write_at(&dir, "b.rs", EMPTY);
    let both = local_run(&dir, &["check", "."], false);
    assert_eq!(both.status.code(), Some(2));
    assert_eq!(both.stdout, ts_only.stdout);
    assert!(String::from_utf8(both.stderr).unwrap().contains("./b.rs"));
}

#[test]
fn corpus_checker_accepts_full_corpus_and_detects_missing_file() {
    let tmp =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("corpus-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    let bin = tmp.join("check_corpus");
    let built = Command::new("rustc")
        .args(["tests/fixtures/check_corpus.rs", "-o"])
        .arg(&bin)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let corpus = "tests/fixtures/empty-test";
    assert!(Command::new(&bin).arg(corpus).status().unwrap().success());
    let copy = tmp.join("copy");
    fs::create_dir_all(&copy).unwrap();
    for e in fs::read_dir(corpus).unwrap() {
        let e = e.unwrap();
        if e.file_name() != "ts_comment_only_clean.ts" {
            fs::copy(e.path(), copy.join(e.file_name())).unwrap();
        }
    }
    let out = Command::new(&bin).arg(&copy).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("ts_comment_only_clean.ts"));
}

const EMPTY_TS: &str = "describe('x', () => {\n  it('a', () => {});\n});\n";
const FULL_TS: &str = "describe('x', () => {\n  it('a', () => { expect(1).toBe(1); });\n});\n";

fn gate(repo: &Repo, dir: &Path, base: &str, rev: &str) -> Output {
    repo.run(dir, &["gate", base, rev])
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

#[test]
fn gate_new_empty_test_fails_and_lists_all_findings_of_grown_file() {
    let repo = Repo::new("gate-empty");
    repo.write("src/a.test.ts", EMPTY_TS.as_bytes());
    repo.write("src/ok.test.ts", FULL_TS.as_bytes());
    let base = repo.commit("base");
    repo.write(
        "src/a.test.ts",
        format!("{EMPTY_TS}it('b', () => {{}});\n").as_bytes(),
    );
    let rev = repo.commit("worse");
    let out = gate(&repo, &repo.root, &base, &rev);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty());
    let expected = "empty-test: не пройдено: було 1, стало 2\n\
        src/a.test.ts:2: empty-test: тест порожній: напишіть перевірку в тілі тесту або видаліть його\n\
        src/a.test.ts:4: empty-test: тест порожній: напишіть перевірку в тілі тесту або видаліть його\n\
        duplication: пройдено: було 0, стало 0\n\
        gate: не пройдено\n";
    assert_eq!(stdout_of(&out), expected);
    assert_eq!(gate(&repo, &repo.root, &base, &rev).stdout, out.stdout);
}

#[test]
fn gate_steady_debt_compensation_and_rename_pass() {
    let repo = Repo::new("gate-pass");
    repo.write("a.test.ts", EMPTY_TS.as_bytes());
    repo.write("b.test.ts", FULL_TS.as_bytes());
    let base = repo.commit("base");
    let same = gate(&repo, &repo.root, &base, &base);
    assert_eq!(same.status.code(), Some(0));
    assert_eq!(
        stdout_of(&same),
        "empty-test: пройдено: було 1, стало 1\nduplication: пройдено: було 0, стало 0\ngate: пройдено\n"
    );
    repo.git(&["mv", "a.test.ts", "c.test.ts"]);
    let renamed = repo.commit("rename");
    assert_eq!(
        gate(&repo, &repo.root, &base, &renamed).status.code(),
        Some(0)
    );
    // Виправлена в одному файлі знахідка компенсує нову в іншому.
    repo.write("c.test.ts", FULL_TS.as_bytes());
    repo.write("b.test.ts", EMPTY_TS.as_bytes());
    let swapped = repo.commit("swap");
    let out = gate(&repo, &repo.root, &base, &swapped);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout_of(&out).contains("empty-test: пройдено: було 1, стало 1\n"));
}

#[test]
fn gate_duplication_growth_shows_partner_and_skips_unrelated_groups() {
    let repo = Repo::new("gate-dup");
    let other = DUP_BLOCK
        .replace("const x", "const xx")
        .replace("x +", "xx +");
    repo.write("a.ts", DUP_BLOCK.as_bytes());
    repo.write("b.ts", other.as_bytes());
    repo.write("c.ts", other.as_bytes());
    let base = repo.commit("base");
    repo.write("d.ts", DUP_BLOCK.as_bytes());
    let rev = repo.commit("more dup");
    let out = gate(&repo, &repo.root, &base, &rev);
    assert_eq!(out.status.code(), Some(1), "{}", stdout_of(&out));
    let text = stdout_of(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "empty-test: пройдено: було 0, стало 0");
    assert!(lines[1].starts_with("duplication: не пройдено: було "));
    assert_eq!(lines[2], "a.ts:1-15: duplication: група 1");
    assert_eq!(lines[3], "d.ts:1-15: duplication: група 1");
    assert_eq!(lines.len(), 5, "{text}");
    assert_eq!(lines[4], "gate: не пройдено");
}

#[test]
fn gate_improvement_of_one_condition_does_not_offset_another() {
    let repo = Repo::new("gate-mixed");
    repo.write("a.ts", DUP_BLOCK.as_bytes());
    repo.write("b.ts", DUP_BLOCK.as_bytes());
    let base = repo.commit("base");
    repo.write("a.ts", b"function a() {}\n");
    repo.write("t.test.ts", EMPTY_TS.as_bytes());
    let rev = repo.commit("mixed");
    let out = gate(&repo, &repo.root, &base, &rev);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout_of(&out);
    assert!(text.contains("empty-test: не пройдено: було 0, стало 1\n"));
    assert!(text.contains("duplication: пройдено: було 30, стало 0\n"));
}

#[test]
fn gate_reads_commits_only_from_any_subdirectory_with_tag_and_unrelated_base() {
    let repo = Repo::new("gate-isolation");
    repo.write("sub/a.test.ts", EMPTY_TS.as_bytes());
    let base = repo.commit("base");
    repo.git(&["tag", "-a", "-m", "t", "v1", "HEAD"]);
    repo.git(&["checkout", "-q", "--orphan", "other"]);
    repo.write("n.test.ts", FULL_TS.as_bytes());
    repo.git(&["rm", "-rq", "--cached", "."]);
    repo.git(&["add", "n.test.ts"]);
    repo.git(&["commit", "-q", "-m", "orphan"]);
    let rev = repo.git(&["rev-parse", "HEAD"]);
    let expected = gate(&repo, &repo.root, "v1", &rev);
    assert_eq!(expected.status.code(), Some(0));
    // Індекс, диск і невідстежувані файли не впливають.
    repo.write("n.test.ts", EMPTY_TS.as_bytes());
    repo.write("untracked.test.ts", EMPTY_TS.as_bytes());
    repo.git(&["add", "n.test.ts"]);
    repo.write("n.test.ts", EMPTY_TS.as_bytes());
    let sub = repo.root.join("sub");
    fs::create_dir_all(&sub).unwrap();
    let again = gate(&repo, &sub, "v1", &rev);
    assert_eq!(again.status.code(), Some(0));
    assert_eq!(again.stdout, expected.stdout);
    assert!(stdout_of(&again).starts_with("empty-test: пройдено: було 1, стало 0\n"));
    assert_eq!(gate(&repo, &repo.root, &base, &base).status.code(), Some(0));
}

#[test]
fn gate_selection_skips_links_gitlinks_and_excluded_directories() {
    let repo = Repo::new("gate-select");
    repo.write("ok.test.ts", FULL_TS.as_bytes());
    let base = repo.commit("base");
    repo.write("node_modules/x.test.ts", EMPTY_TS.as_bytes());
    repo.write("target/t.test.ts", EMPTY_TS.as_bytes());
    repo.write("deep/.git/g.test.ts", EMPTY_TS.as_bytes());
    repo.write("real.test.ts", EMPTY_TS.as_bytes());
    let oid = repo.git(&["hash-object", "-w", "real.test.ts"]);
    fs::remove_file(repo.root.join("real.test.ts")).unwrap();
    repo.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("120000,{oid},link.test.ts"),
    ]);
    repo.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        "160000,1111111111111111111111111111111111111111,sub.test.ts",
    ]);
    repo.git(&["add", "-f", "node_modules", "target", "deep"]);
    let rev = repo.commit("noise");
    let out = gate(&repo, &repo.root, &base, &rev);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout_of(&out).starts_with("empty-test: пройдено: було 0, стало 0\n"));
}

#[test]
fn gate_broken_file_or_bad_revision_is_fatal_with_all_reasons() {
    let repo = Repo::new("gate-fatal");
    repo.write("ok.test.ts", FULL_TS.as_bytes());
    let base = repo.commit("base");
    repo.write("t.test.ts", EMPTY_TS.as_bytes());
    repo.write("bad1.ts", b"function (");
    repo.write("bad2.ts", b"const = ;");
    repo.write("odd\nname.ts", b"let a = 1;\n");
    let rev = repo.commit("broken");
    let out = gate(&repo, &repo.root, &base, &rev);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let err = String::from_utf8(out.stderr).unwrap();
    for name in ["bad1.ts", "bad2.ts", "odd"] {
        assert!(err.contains("ревізія: ") && err.contains(name), "{err}");
    }
    let unknown = gate(&repo, &repo.root, &base, "no-such-rev");
    assert_eq!(unknown.status.code(), Some(2));
    assert!(unknown.stdout.is_empty());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("ревізія"));
}

#[test]
fn gate_stdout_write_failure_is_fatal() {
    let repo = Repo::new("gate-write");
    repo.write("a.test.ts", EMPTY_TS.as_bytes());
    let sha = repo.commit("one");
    let run = || {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
        cmd.args(["gate", &sha, &sha]).current_dir(&repo.root);
        isolate(&mut cmd, &repo.home);
        cmd
    };
    let ok = run().output().unwrap();
    assert_eq!(ok.status.code(), Some(0));
    assert!(!ok.stdout.is_empty());
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut cmd = run();
    cmd.stdout(writer);
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("не вдалося записати звіт: "), "{stderr}");
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Знищує loose-об'єкт blob, щоб файл став непрочитаним у цьому коміті.
fn destroy_blob(repo: &Repo, path: &str, rev: &str) {
    let oid = repo.git(&["rev-parse", &format!("{rev}:{path}")]);
    let obj = repo.loose_object(&oid);
    make_writable(&obj);
    fs::remove_file(obj).unwrap();
}

#[test]
fn gate_fixing_broken_base_file_passes_with_base_warning() {
    let repo = Repo::new("gate-fix-base");
    repo.write("broken.ts", b"function (");
    repo.write("ok.test.ts", FULL_TS.as_bytes());
    let base = repo.commit("base");
    repo.write("broken.ts", b"function f() {}\n");
    let rev = repo.commit("fix");
    let out = gate(&repo, &repo.root, &base, &rev);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert_eq!(
        stdout_of(&out),
        "empty-test: пройдено: було 0, стало 0\nduplication: пройдено: було 0, стало 0\ngate: пройдено\n"
    );
    let err = stderr_of(&out);
    assert!(
        err.contains("база") && err.contains("broken.ts"),
        "{err}"
    );
    // Зламаний файл у ревізії — завжди 2.
    assert_eq!(gate(&repo, &repo.root, &rev, &base).status.code(), Some(2));
}

#[test]
fn gate_partial_base_keeps_known_duplication_sum() {
    let repo = Repo::new("gate-partial-base");
    repo.write("a.ts", DUP_BLOCK.as_bytes());
    repo.write("b.ts", DUP_BLOCK.as_bytes());
    repo.write("c.ts", b"function (");
    let base = repo.commit("base");
    repo.write("c.ts", b"function c() {}\n");
    let fixed = repo.commit("fix");
    let out = gate(&repo, &repo.root, &base, &fixed);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert!(stdout_of(&out).contains("duplication: пройдено: було 30, стало 30\n"));
    assert!(stderr_of(&out).contains("c.ts"));

    // Відсутній blob бази: відома сума лишається, погіршення проти неї дає 1 із місцями.
    repo.write("d.ts", DUP_BLOCK.as_bytes());
    repo.write("c.ts", b"function c() {}\n");
    let worse = repo.commit("worse");
    destroy_blob(&repo, "c.ts", &base);
    let out = gate(&repo, &repo.root, &base, &worse);
    assert_eq!(out.status.code(), Some(1), "{}", stderr_of(&out));
    let text = stdout_of(&out);
    assert!(text.contains("duplication: не пройдено: було 30, стало 45\n"), "{text}");
    assert!(text.contains("d.ts:1-15: duplication: група 1\n"), "{text}");
    assert!(stderr_of(&out).contains("c.ts"));
}

#[test]
fn gate_target_errors_beat_violation_and_name_every_file_with_base_warning() {
    let repo = Repo::new("gate-target-errors");
    repo.write("broken.ts", b"function (");
    let base = repo.commit("base");
    repo.write("t.test.ts", EMPTY_TS.as_bytes());
    repo.write("bad1.ts", b"const = ;");
    repo.write("bad2.ts", b"function (");
    repo.write("odd\nname.ts", b"let a = 1;\n");
    let rev = repo.commit("worse");
    let out = gate(&repo, &repo.root, &base, &rev);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let err = stderr_of(&out);
    assert!(err.contains("попередження: база: broken.ts"), "{err}");
    for name in ["ревізія: bad1.ts", "ревізія: bad2.ts", "ревізія: \"odd"] {
        assert!(err.contains(name), "{err}");
    }
}

/// Запуск gate із PATH, де є лише Git: `rustc` недоступний, а Git справний.
#[cfg(unix)]
fn gate_without_rustc(repo: &Repo, base: &str, rev: &str) -> Output {
    let bin = repo.home.join("git-only");
    fs::create_dir_all(&bin).unwrap();
    let git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|d| d.join("git"))
        .find(|p| p.is_file())
        .expect("git у PATH");
    let link = bin.join("git");
    if !link.exists() {
        std::os::unix::fs::symlink(git, link).unwrap();
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_osavul"));
    cmd.args(["gate", base, rev]).current_dir(&repo.root);
    isolate(&mut cmd, &repo.home);
    cmd.env("PATH", &bin);
    cmd.output().unwrap()
}

#[cfg(unix)]
#[test]
fn gate_needs_rustc_only_when_rust_files_are_selected() {
    let repo = Repo::new("gate-rustc");
    repo.write("ok.test.ts", FULL_TS.as_bytes());
    repo.write("target/x.rs", b"fn main( {");
    let ts_only = repo.commit("ts only");
    let out = gate_without_rustc(&repo, &ts_only, &ts_only);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));

    // Єдиний .rs лише в базі, недекодований: rustc усе одно потрібен.
    repo.git(&["rm", "-rq", "target"]);
    repo.write("lib.rs", b"\xff\xfe");
    let base = repo.commit("bad rs");
    repo.write("lib.rs", b"fn ok() {}\n");
    let rev = repo.commit("fixed rs");
    let out = gate_without_rustc(&repo, &base, &rev);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(stderr_of(&out).contains("rustc"), "{}", stderr_of(&out));

    // За справного rustc виправлення синтаксису дає попередження бази й код за сумами.
    let out = gate(&repo, &repo.root, &base, &rev);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert!(stderr_of(&out).contains("lib.rs"));
}
