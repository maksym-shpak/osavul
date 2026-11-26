mod complexity_report;
mod empty_rust_test;
mod git_tree;
mod local_tree;
mod ts_cyclomatic;

use std::env;
use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

const HELP: &str = "\
osavul — статичний аналізатор коду

Використання:
  osavul --help       показати цю довідку
  osavul --version    показати назву й версію
  osavul check <файл.rs>
                      знайти порожні Rust-тести (правило empty-rust-test)
  osavul complexity <ревізія>
                      цикломатична складність файлів .ts коміту (JSON у stdout);
                      вміст береться з коміту, потрібен локальний Git і запуск
                      усередині репозиторію
  osavul complexity --path <файл-або-тека>
                      те саме для поточного вмісту файла .ts або теки на диску
                      (зокрема незакомічених змін); Git не потрібен

Коди завершення complexity: 0 — повний звіт, 1 — неповний звіт (JSON усе одно
у stdout), 2 — помилка аргументів, Git, коміту, шляху чи обходу (stdout порожній).
Коди завершення check: 0 — знахідок немає, 1 — є знахідки, 2 — помилка.
";

fn main() -> ExitCode {
    // Порівнюємо OsString, щоб аргументи поза UTF-8 теж давали код 2.
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [a] if a == "--help" => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        [a] if a == "--version" => {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        [c, path] if c == "check" => match path.to_str() {
            Some(path) => check(path),
            None => usage_error(),
        },
        [c, flag, path] if c == "complexity" && flag == "--path" => {
            emit(complexity_report::measure_local(std::path::Path::new(path)))
        }
        // Один операнд — завжди ревізія, навіть якщо він схожий на прапорець.
        [c, rev] if c == "complexity" => match rev.to_str() {
            Some(rev) => emit(complexity_report::measure(rev)),
            None => usage_error(),
        },
        _ => usage_error(),
    }
}

fn emit(result: Result<complexity_report::Report, String>) -> ExitCode {
    match result {
        Ok(report) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(e) = stdout.write_all(&report.json).and_then(|()| stdout.flush()) {
                let _ = writeln!(std::io::stderr(), "osavul: не вдалося записати звіт: {e}");
                return ExitCode::from(2);
            }
            ExitCode::from(u8::from(!report.complete))
        }
        Err(reason) => {
            let _ = writeln!(std::io::stderr(), "osavul: {reason}");
            ExitCode::from(2)
        }
    }
}

fn check(path: &str) -> ExitCode {
    let result = std::fs::read_to_string(path)
        .map_err(|e| format!("не вдалося прочитати {path}: {e}"))
        .and_then(|text| {
            empty_rust_test::find_empty_tests(&text).map_err(|e| format!("{path}: {e}"))
        });
    match result {
        Ok(lines) => {
            let mut report = String::new();
            for line in &lines {
                report.push_str(&format!("{path}:{line}: empty-rust-test\n"));
            }
            let _ = std::io::stdout().write_all(report.as_bytes());
            ExitCode::from(u8::from(!lines.is_empty()))
        }
        Err(reason) => {
            let _ = writeln!(std::io::stderr(), "osavul: {reason}");
            ExitCode::from(2)
        }
    }
}

fn usage_error() -> ExitCode {
    let _ = writeln!(
        std::io::stderr(),
        "osavul: непідтримувані аргументи; див. `osavul --help`"
    );
    ExitCode::from(2)
}
