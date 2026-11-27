//! Рушій `osavul check`: інвентар файлів, мова за суфіксом, запуск застосовних правил, сортування,
//! вивід і код завершення. Що саме є знахідкою, вирішують правила; рушій знає лише позиції.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use crate::empty_test::EMPTY_TEST;
use crate::local_tree::{self, InputKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    TypeScript,
}

/// Мовна реалізація: вміст одного файла й ім'я для режиму розбору → фізичні рядки від 1 (з
/// повторами) або причина відмови файла.
pub type Detector = fn(&[u8], &str) -> Result<Vec<usize>, String>;

/// Правило: один ідентифікатор і повідомлення для всіх мов та реалізація для кожної мови.
pub struct Rule {
    pub id: &'static str,
    pub message: &'static str,
    pub detectors: &'static [(Lang, Detector)],
}

/// Статичний перелік правил, вбудований у бінарник.
const RULES: &[&Rule] = &[&EMPTY_TEST];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: Vec<u8>,
    pub line: usize,
    pub rule: &'static str,
    pub message: &'static str,
}

fn language_of(key: &[u8]) -> Option<Lang> {
    if key.ends_with(b".rs") {
        Some(Lang::Rust)
    } else if key.ends_with(b".ts") {
        Some(Lang::TypeScript)
    } else {
        None
    }
}

/// Сортує за байтами шляху, номером рядка й ідентифікатором правила та формує текст звіту.
/// Повтори зберігаються.
pub fn render(findings: &mut [Finding]) -> Vec<u8> {
    findings.sort_by(|a, b| (&a.path, a.line, a.rule).cmp(&(&b.path, b.line, b.rule)));
    let mut out = Vec::new();
    for f in findings.iter() {
        out.extend_from_slice(&f.path);
        out.extend_from_slice(format!(":{}: {}: {}\n", f.line, f.rule, f.message).as_bytes());
    }
    out
}

/// Шлях знахідки: для явного файла — дослівний аргумент, для теки — аргумент, `/` (якщо аргумент
/// ще не закінчується роздільником) і ключ файла.
fn printed_path(arg: &str, kind: InputKind, key: &[u8]) -> Result<Vec<u8>, String> {
    let mut path = arg.as_bytes().to_vec();
    if kind == InputKind::Directory {
        let sep_native = cfg!(windows) && path.last() == Some(&b'\\');
        if path.last() != Some(&b'/') && !sep_native {
            // «C:» + «/x» змінило б адресу з відносної в діску на кореневу.
            if cfg!(windows) && path.last() == Some(&b':') {
                return Err(
                    "корінь із завершальним `:` не можна поєднати з відносним ключем".into(),
                );
            }
            path.push(b'/');
        }
        if cfg!(windows) && key.contains(&0xEF) && String::from_utf8_lossy(key).contains('\u{fffd}')
        {
            return Err("ім'я файла не має точного UTF-8 подання".into());
        }
        path.extend_from_slice(key);
    }
    if path.contains(&b'\n') {
        return Err("шлях містить перенесення рядка й не вміщується в один запис".into());
    }
    Ok(path)
}

fn shown(path: &[u8]) -> String {
    let s = String::from_utf8_lossy(path);
    if s.contains('\n') || s.contains('\u{fffd}') {
        format!("{s:?}")
    } else {
        s.into_owned()
    }
}

/// Виконує перевірку й повертає код завершення; stdout і stderr передаються окремо для тестів.
pub fn execute(arg: &str, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let (kind, selected) = match local_tree::select(Path::new(arg), &local_tree::CHECK) {
        Ok(v) => v,
        Err(reason) => {
            let _ = writeln!(stderr, "osavul: {reason}");
            return 2;
        }
    };
    let mut findings = Vec::new();
    let mut failed = false;
    for file in &selected {
        let label = printed_path(arg, kind, &file.key);
        let name = shown(label.as_deref().unwrap_or(arg.as_bytes()));
        let result = check_file(&file.disk, &file.key);
        let positions = match result {
            Ok(p) => p,
            Err(reason) => {
                failed = true;
                let _ = writeln!(stderr, "osavul: {name}: {reason}");
                continue;
            }
        };
        if positions.is_empty() {
            continue;
        }
        match label {
            Ok(path) => findings.extend(positions.into_iter().map(|(rule, line)| Finding {
                path: path.clone(),
                line,
                rule: rule.id,
                message: rule.message,
            })),
            Err(reason) => {
                failed = true;
                let _ = writeln!(stderr, "osavul: {name}: {reason}");
            }
        }
    }
    let report = render(&mut findings);
    if let Err(e) = stdout.write_all(&report).and_then(|()| stdout.flush()) {
        let _ = writeln!(stderr, "osavul: не вдалося записати звіт: {e}");
        return 2;
    }
    if failed {
        2
    } else {
        u8::from(!findings.is_empty())
    }
}

/// Читає файл і запускає всі застосовні правила; успіх лише коли успішні всі.
fn check_file(disk: &Path, key: &[u8]) -> Result<Vec<(&'static Rule, usize)>, String> {
    let source = std::fs::read(disk).map_err(|e| format!("не вдалося прочитати: {e}"))?;
    let Some(lang) = language_of(key) else {
        return Ok(Vec::new());
    };
    let name = String::from_utf8_lossy(key);
    let mut all = Vec::new();
    for rule in RULES {
        for (l, detect) in rule.detectors {
            if *l == lang {
                for line in detect(&source, &name)? {
                    all.push((*rule, line));
                }
            }
        }
    }
    Ok(all)
}

pub fn run(arg: &str) -> ExitCode {
    ExitCode::from(execute(
        arg,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, line: usize, rule: &'static str) -> Finding {
        Finding {
            path: path.as_bytes().to_vec(),
            line,
            rule,
            message: "m",
        }
    }

    // Виявляє зміну ключів порядку (байти шляху, числовий рядок, ідентифікатор) і втрату повторів.
    #[test]
    fn orders_by_path_bytes_numeric_line_rule_and_keeps_duplicates() {
        let mut v = vec![
            f("b", 1, "a-rule"),
            f("a/x", 2, "z-rule"),
            f("a", 10, "a-rule"),
            f("a", 9, "z-rule"),
            f("a", 9, "a-rule"),
            f("a", 9, "a-rule"),
        ];
        let text = String::from_utf8(render(&mut v)).unwrap();
        assert_eq!(
            text,
            "a:9: a-rule: m\na:9: a-rule: m\na:9: z-rule: m\na:10: a-rule: m\na/x:2: z-rule: m\nb:1: a-rule: m\n"
        );
    }

    #[test]
    fn printed_path_joins_directory_without_doubling_separator() {
        let p = |a: &str, k: &str, kind| printed_path(a, kind, k.as_bytes()).unwrap();
        assert_eq!(p("./src", "a/b.rs", InputKind::Directory), b"./src/a/b.rs");
        assert_eq!(p("./src/", "a.rs", InputKind::Directory), b"./src/a.rs");
        assert_eq!(p("./x.rs", "x.rs", InputKind::File), b"./x.rs");
        assert!(printed_path("d", InputKind::Directory, b"a\nb.rs").is_err());
        assert!(printed_path("a\nb.rs", InputKind::File, b"b.rs").is_err());
    }
}
