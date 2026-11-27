//! Точка входу CLI: розбирає аргументи, передає роботу модулям аналізу та перетворює результат на
//! вивід і код виходу. Саму аналітику й читання джерел модуль не виконує.

mod check;
mod complexity_report;
mod coverage_report;
mod duplication_report;
mod empty_rust_test;
mod empty_test;
mod git_tree;
mod lcov;
mod local_tree;
mod ts_cognitive;
mod ts_coverage;
mod ts_cyclomatic;
mod ts_duplication;
mod ts_loc;
mod ts_source;

use std::env;
use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use complexity_report::Metric;

const HELP: &str = "\
osavul — статичний аналізатор коду

Використання:
  osavul --help       показати цю довідку
  osavul --version    показати назву й версію
  osavul check <файл-або-тека>
                      знайти порожні тести (правило empty-test) у поточному
                      вмісті файла .ts (і .d.ts), .rs чи теки на диску (рекурсивно, без
                      .git, node_modules і вкладених target); потрібен
                      локальний rustc, лише якщо відібрано файли .rs; файл
                      з іншим суфіксом — помилка
  osavul complexity <ревізія> [--metric <cyclomatic|cognitive|loc>]
                      складність файлів .ts коміту (JSON у stdout);
                      вміст береться з коміту, потрібен локальний Git і запуск
                      усередині репозиторію
  osavul complexity --path <файл-або-тека> [--metric <cyclomatic|cognitive|loc>]
                      те саме для поточного вмісту файла .ts або теки на диску
                      (зокрема незакомічених змін); Git не потрібен

  osavul duplication <ревізія>
                      дублювання коду у файлах .ts коміту (JSON у stdout,
                      методика ts-duplication); потрібен локальний Git
  osavul duplication --path <файл-або-тека>
                      те саме для поточного вмісту на диску; Git не потрібен

  osavul coverage <ревізія> --lcov <файл> [--lcov <файл>…] [--tests <шлях>…]
                      покриття рядків і гілок у файлах .ts коміту з готових
                      звітів LCOV (JSON у stdout, методика ts-coverage);
                      тести не запускаються; потрібен локальний Git
  osavul coverage --path <тека> --lcov <файл> [--lcov <файл>…] [--tests <шлях>…]
                      те саме для поточного вмісту теки; Git не потрібен

Опція --metric (лише в кінці виклику) обирає методику: cyclomatic (типово,
ts-cyclomatic) cognitive (ts-cognitive) або loc (ts-loc, розмір коду: рядки з кодом без
порожніх і коментарів); у записах файлів поле називається так само, як метрика.

Коди завершення complexity: 0 — повний звіт, 1 — неповний звіт (JSON усе одно
у stdout), 2 — помилка аргументів, Git, коміту, шляху чи обходу (stdout порожній).
Коди завершення duplication: 0 — повний звіт, 1 — неповний звіт (JSON у stdout;
числа успішних файлів — нижня межа покриття), 2 — помилка аргументів, Git,
коміту чи шляху (stdout порожній). Наявність дублів на код не впливає.
Коди завершення coverage: 0 — повний звіт, 1 — неповний звіт (JSON у stdout),
2 — помилка аргументів, Git, шляху чи читання LCOV (stdout порожній).
--tests — шляхи від кореня (ревізії чи теки), що є тестами й не входять у
покриття; низьке покриття на код не впливає.
Коди завершення check: 0 — усі файли перевірено, знахідок немає, 1 — є знахідки,
2 — помилка (має пріоритет). Помилка аргументів, шляху чи переліку теки: stdout
порожній; помилка окремого файла: його знахідок немає, решта друкується, а
шлях і причина йдуть у stderr. Запис: <шлях>:<рядок>: empty-test: <повідомлення>.
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
            Some(path) => check::run(path),
            None => usage_error(),
        },
        [c, flag, path] if c == "complexity" && flag == "--path" => emit(
            complexity_report::measure_local(std::path::Path::new(path), Metric::Cyclomatic),
        ),
        [c, flag, path, opt, value] if c == "complexity" && flag == "--path" => {
            match parse_metric(opt, value) {
                Some(metric) => emit(complexity_report::measure_local(
                    std::path::Path::new(path),
                    metric,
                )),
                None => usage_error(),
            }
        }
        // Один операнд — завжди ревізія, навіть якщо він схожий на прапорець.
        [c, rev] if c == "complexity" => match rev.to_str() {
            Some(rev) => emit(complexity_report::measure(rev, Metric::Cyclomatic)),
            None => usage_error(),
        },
        [c, rev, opt, value] if c == "complexity" => match (rev.to_str(), parse_metric(opt, value))
        {
            (Some(rev), Some(metric)) => emit(complexity_report::measure(rev, metric)),
            _ => usage_error(),
        },
        [c, flag, path] if c == "duplication" && flag == "--path" => emit(
            duplication_report::measure_local(std::path::Path::new(path)),
        ),
        [c, rev] if c == "duplication" => match rev.to_str() {
            Some(rev) => emit(duplication_report::measure(rev)),
            None => usage_error(),
        },
        [c, rest @ ..] if c == "coverage" => coverage(rest),
        _ => usage_error(),
    }
}

/// Розбирає аргументи `coverage`: селектор джерела (`--path <тека>` або ревізія), далі `--lcov` з
/// одним операндом на кожну появу та `--tests` з одним або кількома операндами.
///
/// Операнди `--tests` закінчуються на першому аргументі, що починається з `--`. Хоча б один
/// `--lcov` обов'язковий; будь-яка інша форма дає код 2.
fn coverage(args: &[OsString]) -> ExitCode {
    let (path_mode, source, rest) = match args {
        [flag, path, rest @ ..] if flag == "--path" => (true, path, rest),
        [rev, rest @ ..] => (false, rev, rest),
        [] => return usage_error(),
    };
    let mut lcov: Vec<&std::ffi::OsStr> = Vec::new();
    let mut tests: Vec<&std::ffi::OsStr> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--lcov" {
            match rest.get(i + 1) {
                Some(v) => lcov.push(v),
                None => return usage_error(),
            }
            i += 2;
        } else if rest[i] == "--tests" {
            let start = i + 1;
            i = start;
            while i < rest.len() && !rest[i].to_string_lossy().starts_with("--") {
                tests.push(&rest[i]);
                i += 1;
            }
            if i == start {
                return usage_error();
            }
        } else {
            return usage_error();
        }
    }
    if lcov.is_empty() {
        return usage_error();
    }
    let tests = match coverage_report::parse_tests(&tests) {
        Ok(t) => t,
        Err(reason) => {
            let _ = writeln!(std::io::stderr(), "osavul: {reason}; див. `osavul --help`");
            return ExitCode::from(2);
        }
    };
    if path_mode {
        emit(coverage_report::measure_local(
            std::path::Path::new(source),
            &lcov,
            tests,
        ))
    } else {
        match source.to_str() {
            Some(rev) => emit(coverage_report::measure(rev, &lcov, tests)),
            None => usage_error(),
        }
    }
}

fn parse_metric(opt: &OsString, value: &OsString) -> Option<Metric> {
    if opt != "--metric" {
        return None;
    }
    match value.to_str()? {
        "cyclomatic" => Some(Metric::Cyclomatic),
        "cognitive" => Some(Metric::Cognitive),
        "loc" => Some(Metric::Loc),
        _ => None,
    }
}

/// Друкує готовий звіт у `stdout` і повертає код виходу: 0 для повного звіту, 1 для неповного, 2
/// для помилки; тоді `stdout` порожній, а причина йде в `stderr`.
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

fn usage_error() -> ExitCode {
    let _ = writeln!(
        std::io::stderr(),
        "osavul: непідтримувані аргументи; див. `osavul --help`"
    );
    ExitCode::from(2)
}
