use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_osavul"))
        .args(args)
        .output()
        .expect("не вдалося запустити osavul")
}

fn assert_usage_error(args: &[&str]) {
    let out = run(args);
    assert_eq!(out.status.code(), Some(2), "аргументи: {args:?}");
    assert!(out.stdout.is_empty(), "stdout має бути порожнім: {args:?}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("osavul --help"), "stderr: {stderr:?}");
}

#[test]
fn help_is_stable_and_lists_both_invocations() {
    let first = run(&["--help"]);
    let second = run(&["--help"]);
    for out in [&first, &second] {
        assert_eq!(out.status.code(), Some(0));
        assert!(out.stderr.is_empty());
    }
    let text = String::from_utf8(first.stdout.clone()).unwrap();
    assert!(text.contains("--help") && text.contains("--version"));
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
fn check_is_usage_error() {
    assert_usage_error(&["check"]);
}

#[test]
fn extra_argument_is_usage_error() {
    assert_usage_error(&["--version", "extra"]);
}
