//! Структурний перевірник корпусу. Використання: check_corpus <каталог корпусу>
//! Лише стандартна бібліотека. Номер рядка (`fn` для Rust, ідентифікатор виклику для TypeScript) і зміст звіряють вручну.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::exit;

const PAIRS: [&str; 5] = [
    "empty-body",
    "comment-only",
    "no-test-attr",
    "ts-direct-call",
    "ts-comment-only",
];

fn is_source(p: &Path) -> bool {
    p.extension().map_or(false, |x| x == "rs" || x == "ts")
}

fn lang(path: &str) -> &'static str {
    if path.ends_with(".ts") { "ts" } else { "rs" }
}

fn collect(root: &Path, dir: &Path, out: &mut BTreeSet<String>) -> Result<(), String> {
    let rd = fs::read_dir(dir).map_err(|e| format!("не вдалося прочитати каталог {}: {e}", dir.display()))?;
    for entry in rd {
        let p = entry.map_err(|e| e.to_string())?.path();
        if p.is_dir() {
            collect(root, &p, out)?;
        } else if is_source(&p) {
            let rel = p.strip_prefix(root).unwrap();
            let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            out.insert(parts.join("/"));
        }
    }
    Ok(())
}

fn run(root: &Path) -> Vec<String> {
    let mut errs = Vec::new();
    let text = match fs::read(root.join("expected.tsv")).ok().and_then(|b| String::from_utf8(b).ok()) {
        Some(t) => t,
        None => return vec!["expected.tsv: не вдалося прочитати як UTF-8".into()],
    };
    let mut lines = text.lines();
    if lines.next() != Some("path\tpair\tlines") {
        errs.push("expected.tsv: заголовок має бути «path<TAB>pair<TAB>lines»".to_string());
    }
    // path -> (pair, Some(N) | None для [])
    let mut entries: BTreeMap<String, (String, Option<usize>)> = BTreeMap::new();
    for (i, line) in lines.enumerate() {
        let n = i + 2;
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 3 || f.iter().take(2).any(|s| s.is_empty()) {
            errs.push(format!("expected.tsv:{n}: запис має містити три поля path, pair, lines: {line:?}"));
            continue;
        }
        let lines_val = if f[2] == "[]" {
            None
        } else if let Some(d) = f[2].strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            match d.parse::<usize>() {
                Ok(v) if !d.starts_with('+') => Some(v),
                _ => {
                    errs.push(format!("expected.tsv:{n}: недійсне поле lines {:?} для {}", f[2], f[0]));
                    continue;
                }
            }
        } else {
            errs.push(format!("expected.tsv:{n}: недійсне поле lines {:?} для {}", f[2], f[0]));
            continue;
        };
        if entries.contains_key(f[0]) {
            errs.push(format!("expected.tsv:{n}: повтор шляху {}", f[0]));
            continue;
        }
        entries.insert(f[0].to_string(), (f[1].to_string(), lines_val));
    }

    let mut files = BTreeSet::new();
    if let Err(e) = collect(root, root, &mut files) {
        errs.push(e);
        return errs;
    }
    for f in &files {
        if !entries.contains_key(f) {
            errs.push(format!("файл {f} не має запису в expected.tsv"));
        }
    }
    for p in entries.keys() {
        if !files.contains(p) {
            errs.push(format!("запис {p} не має файла"));
        }
    }

    for (p, (_, v)) in &entries {
        if let (Some(v), true) = (v, files.contains(p)) {
            let count = fs::read_to_string(root.join(p)).map(|s| s.lines().count()).unwrap_or(0);
            if *v < 1 || *v > count {
                errs.push(format!("{p}: номер рядка {v} поза діапазоном 1..={count}"));
            }
        }
    }

    let mut by_pair: BTreeMap<&str, Vec<(&str, &Option<usize>)>> = BTreeMap::new();
    for (p, (pair, v)) in &entries {
        by_pair.entry(pair.as_str()).or_default().push((p, v));
    }
    for pair in by_pair.keys() {
        if !PAIRS.contains(pair) {
            errs.push(format!("пара {pair}: невідомий ідентифікатор"));
        }
    }
    for pair in PAIRS {
        match by_pair.get(pair) {
            None => errs.push(format!("пара {pair}: відсутня")),
            Some(v) => {
                let found = v.iter().filter(|(_, l)| l.is_some()).count();
                let same_lang = v.iter().all(|(p, _)| lang(p) == lang(v[0].0));
                if !same_lang {
                    errs.push(format!("пара {pair}: файли пари мають різні мови"));
                }
                if v.len() != 2 || found != 1 {
                    errs.push(format!(
                        "пара {pair}: потрібні два записи — один [N] і один [], а є {} записів, з них {found} зі знахідкою",
                        v.len()
                    ));
                }
            }
        }
    }
    errs
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("використання: {} <каталог корпусу>", args[0]);
        exit(2);
    }
    let errs = run(Path::new(&args[1]));
    if errs.is_empty() {
        println!("корпус узгоджений");
    } else {
        for e in &errs {
            eprintln!("помилка: {e}");
        }
        exit(1);
    }
}
