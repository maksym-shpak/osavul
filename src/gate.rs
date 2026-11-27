//! Quality gate: порівнює суми правил `check` і `duplicatedLines` двох комітів та будує текстовий
//! вердикт із місцями порушень. Аналіз виконують рушій правил і модуль дублювання; тут лише
//! порівняння, відбір подробиць і формат виводу.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::check::{self, Finding, Rule};
use crate::complexity_report::FileInput;
use crate::duplication_report::{self, Analysis};
use crate::git_tree;

/// Готовий результат порівняння: повний stdout, код 0 або 1 і попередження про файли бази.
pub struct Verdict {
    pub stdout: Vec<u8>,
    pub code: u8,
    pub warnings: Vec<String>,
}

struct Found {
    key: Vec<u8>,
    rule: &'static Rule,
    line: usize,
}

/// Усе, що відомо про один бік порівняння.
struct Side {
    findings: Vec<Found>,
    dup_keys: Vec<Vec<u8>>,
    dup: Analysis,
}

/// Вхідні дані одного боку до аналізу.
struct Raw {
    label: &'static str,
    inv: git_tree::GateInventory,
    contents: Vec<Result<Vec<u8>, git_tree::ReadError>>,
}

impl Raw {
    fn needs_rust(&self) -> bool {
        // Рішення залежить лише від відібраних ключів, а не від того, чи вдалося їх прочитати.
        self.inv
            .entries
            .iter()
            .zip(&self.inv.selections)
            .any(|(e, s)| s.check && check::uses_rust(&e.path))
    }
}

/// Порівнює два коміти. `Err` — фатальна відмова: кожен елемент є рядком діагностики
/// (попередження бази, якщо вони вже встановлені, ідуть першими).
pub fn compare_commits(base: &str, rev: &str) -> Result<Verdict, Vec<String>> {
    let base_raw = collect("база", base);
    let target_raw = collect("ревізія", rev);
    let (base_raw, target_raw) = match (base_raw, target_raw) {
        (Ok(b), Ok(t)) => (b, t),
        (b, t) => {
            let mut errors = Vec::new();
            errors.extend(b.err());
            errors.extend(t.err());
            return Err(errors);
        }
    };

    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    // Єдиний `.rs` у базі може не дійти до rustc (не читається чи не декодується), тому
    // працездатність потрібного режиму перевіряється окремо, на коректному тексті.
    let rust_ok = if base_raw.needs_rust() || target_raw.needs_rust() {
        match check::probe_rust() {
            Ok(()) => true,
            Err(e) => {
                errors.push(e.reason);
                false
            }
        }
    } else {
        true
    };
    let base_side = analyze(&base_raw, rust_ok, &mut warnings, &mut errors);
    let target_side = analyze(&target_raw, rust_ok, &mut warnings, &mut errors);
    if !errors.is_empty() {
        let mut lines = warnings;
        lines.extend(errors);
        return Err(lines);
    }
    let mut verdict = compare(&base_side, &target_side);
    verdict.warnings = warnings;
    Ok(verdict)
}

fn collect(label: &'static str, rev: &str) -> Result<Raw, String> {
    let inv = git_tree::gate_inventory(rev).map_err(|e| format!("{label}: {e}"))?;
    let contents = git_tree::read_contents(&inv.entries);
    Ok(Raw {
        label,
        inv,
        contents,
    })
}

/// Причина відмови файла: помилка самого файла чи недоступність інструмента.
struct Failure {
    tool: bool,
    reason: String,
}

fn analyze(
    raw: &Raw,
    rust_ok: bool,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> Side {
    let is_base = raw.label == "база";
    // Один файл дає один запис незалежно від того, скільки умов його відхилили.
    let mut failures: BTreeMap<Vec<u8>, Failure> = BTreeMap::new();
    let mut fail = |key: &Vec<u8>, tool: bool, reason: String| {
        let slot = failures.entry(key.clone());
        match slot {
            std::collections::btree_map::Entry::Vacant(v) => {
                v.insert(Failure { tool, reason });
            }
            std::collections::btree_map::Entry::Occupied(mut o) => {
                if tool && !o.get().tool {
                    o.insert(Failure { tool, reason });
                }
            }
        }
    };
    let mut findings = Vec::new();
    let mut dup_keys = Vec::new();
    let mut dup_inputs = Vec::new();
    let rows = raw.inv.entries.iter().zip(&raw.inv.selections);
    for ((entry, selection), content) in rows.zip(&raw.contents) {
        let key = &entry.path;
        // Для бази достатньо байтового ключа: неподатний шлях не заважає підрахунку.
        if !is_base && let Err(reason) = check::commit_path(key) {
            fail(key, false, reason);
        }
        if selection.check {
            match content {
                Ok(_) if !rust_ok && check::uses_rust(key) => {}
                Ok(bytes) => match check::check_bytes(key, bytes) {
                    Ok(positions) => {
                        findings.extend(positions.into_iter().map(|(rule, line)| Found {
                            key: key.clone(),
                            rule,
                            line,
                        }))
                    }
                    Err(e) => fail(key, e.kind == check::FailureKind::Tool, e.reason),
                },
                Err(e) => fail(key, false, e.reason().to_string()),
            }
        }
        if selection.dup {
            dup_keys.push(key.clone());
            dup_inputs.push(FileInput {
                path: key.clone(),
                content: match content {
                    Ok(b) => Ok(b.clone()),
                    Err(_) => Err(git_tree::ReadError),
                },
            });
        }
    }
    let dup = duplication_report::analyze(&dup_inputs);
    for (key, outcome) in dup_keys.iter().zip(&dup.files) {
        if let Err(reason) = outcome {
            fail(key, false, (*reason).to_string());
        }
    }
    for (key, f) in failures {
        let name = check::shown(&key);
        if is_base && !f.tool {
            warnings.push(format!(
                "попередження: {}: {name}: {} (файл бази: його внесок рахується за тим, що вдалося визначити)",
                raw.label, f.reason
            ));
        } else {
            errors.push(format!("{}: {name}: {}", raw.label, f.reason));
        }
    }
    Side {
        findings,
        dup_keys,
        dup,
    }
}

fn state(violated: bool) -> &'static str {
    if violated {
        "не пройдено"
    } else {
        "пройдено"
    }
}

fn compare(base: &Side, target: &Side) -> Verdict {
    let mut out = Vec::new();
    let mut failed = false;

    let mut rules: Vec<&'static Rule> = check::rules().to_vec();
    rules.sort_by_key(|r| r.id);
    for rule in rules {
        let count = |side: &Side| {
            side.findings
                .iter()
                .filter(|f| f.rule.id == rule.id)
                .count()
        };
        let (was, now) = (count(base), count(target));
        let violated = now > was;
        failed |= violated;
        out.extend_from_slice(
            format!(
                "{}: {}: було {was}, стало {now}\n",
                rule.id,
                state(violated)
            )
            .as_bytes(),
        );
        if violated {
            out.extend(rule_details(rule, base, target));
        }
    }

    let (was, now) = (base.dup.duplicated_lines, target.dup.duplicated_lines);
    let violated = now > was;
    failed |= violated;
    out.extend_from_slice(
        format!(
            "duplication: {}: було {was}, стало {now}\n",
            state(violated)
        )
        .as_bytes(),
    );
    if violated {
        out.extend(duplication_details(base, target));
    }

    out.extend_from_slice(format!("gate: {}\n", state(failed)).as_bytes());
    Verdict {
        stdout: out,
        code: u8::from(failed),
        warnings: Vec::new(),
    }
}

/// Усі знахідки правила в цілі з тих файлів, де їх побільшало проти того самого ключа в базі.
fn rule_details(rule: &Rule, base: &Side, target: &Side) -> Vec<u8> {
    fn per_key<'a>(side: &'a Side, id: &str) -> HashMap<&'a [u8], usize> {
        let mut map: HashMap<&[u8], usize> = HashMap::new();
        for f in side.findings.iter().filter(|f| f.rule.id == id) {
            *map.entry(f.key.as_slice()).or_default() += 1;
        }
        map
    }
    let (was, now) = (per_key(base, rule.id), per_key(target, rule.id));
    let mut shown: Vec<Finding> = target
        .findings
        .iter()
        .filter(|f| f.rule.id == rule.id)
        .filter(|f| now[f.key.as_slice()] > was.get(f.key.as_slice()).copied().unwrap_or(0))
        .map(|f| Finding {
            path: f.key.clone(),
            line: f.line,
            rule: f.rule.id,
            message: f.rule.message,
        })
        .collect();
    check::render(&mut shown)
}

/// Кожна група цілі, що має входження у файлі зі зростанням `duplicatedLines`, з усіма
/// входженнями. Номери йдуть поспіль лише за показаними групами.
fn duplication_details(base: &Side, target: &Side) -> Vec<u8> {
    let was: HashMap<&[u8], u64> = base
        .dup_keys
        .iter()
        .zip(&base.dup.files)
        .filter_map(|(k, c)| c.as_ref().ok().map(|c| (k.as_slice(), c.duplicated_lines)))
        .collect();
    let grown: HashSet<usize> = target
        .dup
        .files
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let now = c.as_ref().ok()?.duplicated_lines;
            let before = was.get(target.dup_keys[i].as_slice()).copied().unwrap_or(0);
            (now > before).then_some(i)
        })
        .collect();
    let mut out = Vec::new();
    let mut number = 0;
    for group in &target.dup.groups {
        if !group.iter().any(|(file, _, _)| grown.contains(file)) {
            continue;
        }
        number += 1;
        for &(file, start, end) in group {
            out.extend_from_slice(&target.dup_keys[file]);
            out.extend_from_slice(
                format!(":{start}-{end}: duplication: група {number}\n").as_bytes(),
            );
        }
    }
    out
}
