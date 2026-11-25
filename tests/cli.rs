use std::fs;
use std::path::PathBuf;
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
