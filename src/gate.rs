//! Quality gate: порівнює суми правил `check` і `duplicatedLines` двох комітів та будує текстовий
//! вердикт із місцями порушень. Аналіз виконують рушій правил і модуль дублювання; тут лише
//! порівняння, відбір подробиць і формат виводу.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::check::{self, Finding, Rule};
use crate::complexity_report::FileInput;
use crate::duplication_report::{self, Analysis};
use crate::git_tree;

/// Готовий результат порівняння: повний stdout і код 0 або 1.
pub struct Verdict {
    pub stdout: Vec<u8>,
    pub code: u8,
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

/// Порівнює два коміти. `Err` — фатальна відмова: кожен елемент є рядком діагностики.
pub fn compare_commits(base: &str, rev: &str) -> Result<Verdict, Vec<String>> {
    let base_side = load("база", base);
    let target_side = load("ревізія", rev);
    match (base_side, target_side) {
        (Ok(base), Ok(target)) => Ok(compare(&base, &target)),
        (b, t) => {
            let mut errors = Vec::new();
            errors.extend(b.err().unwrap_or_default());
            errors.extend(t.err().unwrap_or_default());
            Err(errors)
        }
    }
}

fn load(label: &str, rev: &str) -> Result<Side, Vec<String>> {
    let inv = git_tree::gate_inventory(rev).map_err(|e| vec![format!("{label}: {e}")])?;
    let contents = git_tree::read_contents(&inv.entries);

    // Один файл дає один запис незалежно від того, скільки умов його відхилили.
    let mut errors: BTreeMap<Vec<u8>, String> = BTreeMap::new();
    let mut findings = Vec::new();
    let mut dup_keys = Vec::new();
    let mut dup_inputs = Vec::new();
    for ((entry, selection), content) in inv.entries.iter().zip(&inv.selections).zip(contents) {
        let key = &entry.path;
        if let Err(reason) = check::commit_path(key) {
            errors.entry(key.clone()).or_insert(reason);
        }
        if selection.check {
            match &content {
                Ok(bytes) => match check::check_bytes(key, bytes) {
                    Ok(positions) => {
                        findings.extend(positions.into_iter().map(|(rule, line)| Found {
                            key: key.clone(),
                            rule,
                            line,
                        }))
                    }
                    Err(e) => {
                        errors.entry(key.clone()).or_insert(e.reason);
                    }
                },
                Err(e) => {
                    errors.entry(key.clone()).or_insert(e.reason().to_string());
                }
            }
        }
        if selection.dup {
            dup_keys.push(key.clone());
            dup_inputs.push(FileInput {
                path: key.clone(),
                content,
            });
        }
    }
    let dup = duplication_report::analyze(&dup_inputs);
    for (key, outcome) in dup_keys.iter().zip(&dup.files) {
        if let Err(reason) = outcome {
            errors.entry(key.clone()).or_insert((*reason).to_string());
        }
    }
    if errors.is_empty() {
        Ok(Side {
            findings,
            dup_keys,
            dup,
        })
    } else {
        Err(errors
            .into_iter()
            .map(|(key, reason)| format!("{label}: {}: {reason}", check::shown(&key)))
            .collect())
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
