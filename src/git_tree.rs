//! Інвентар і вміст `.ts`-файлів зафіксованого коміту через локальний Git.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use crate::local_tree;

/// Вміст об'єкта не вдалося отримати: його немає, він не `blob` або вивід Git обірвано.
#[derive(Debug, PartialEq, Eq)]
pub struct ReadError;

impl ReadError {
    /// Стабільна причина для звіту.
    pub fn reason(&self) -> &'static str {
        "read-error"
    }
}

#[derive(Debug)]
pub struct Entry {
    /// Вихідні байти шляху від кореня Git-дерева.
    pub path: Vec<u8>,
    /// Ідентифікатор `blob`, за яким читається вміст.
    pub oid: String,
}

#[derive(Debug)]
pub struct Inventory {
    /// Повний SHA коміту, до якого розв'язано ревізію.
    pub sha: String,
    /// Упорядковано за байтами шляху.
    pub entries: Vec<Entry>,
}

fn git() -> Command {
    let mut cmd = Command::new("git");
    // Підміна об'єктів (refs/replace) не повинна змінювати виміряний коміт.
    cmd.arg("--no-replace-objects")
        .env("GIT_NO_REPLACE_OBJECTS", "1");
    // У частковому клоні відсутній об'єкт має бути помилкою читання, а не мережевим довантаженням.
    cmd.env("GIT_NO_LAZY_FETCH", "1");
    cmd
}

/// Виконує команду Git без `stdin` і повертає `stdout`; при невдачі помилка містить `stderr` із
/// префіксом `what`.
fn run(mut cmd: Command, what: &str) -> Result<Vec<u8>, String> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("не вдалося запустити git: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let msg = String::from_utf8_lossy(&out.stderr);
        Err(format!("{what}: {}", msg.trim()))
    }
}

/// Абсолютна база зіставлення зовнішніх звітів: верхня тека робочого дерева, а для bare — тека
/// самого репозиторію. Байти імені зберігаються, знімається лише службовий LF.
pub fn match_root() -> Result<Vec<u8>, String> {
    let mut cmd = git();
    cmd.args(["rev-parse", "--is-bare-repository"]);
    let bare = run(cmd, "не вдалося визначити тип репозиторію")?;
    let mut cmd = git();
    if bare.trim_ascii() == b"true" {
        cmd.args(["rev-parse", "--absolute-git-dir"]);
    } else {
        cmd.args(["rev-parse", "--show-toplevel"]);
    }
    let mut out = run(cmd, "не вдалося визначити корінь репозиторію")?;
    if out.last() == Some(&b'\n') {
        out.pop();
    }
    if out.is_empty() {
        return Err("не вдалося визначити корінь репозиторію: порожня відповідь".to_string());
    }
    Ok(out)
}

/// Розв'язує ревізію до повного SHA коміту й повертає впорядкований інвентар
/// звичайних файлів `.ts` від кореня дерева цього коміту.
pub fn inventory(rev: &str) -> Result<Inventory, String> {
    let sha = resolve_commit(rev)?;
    let entries = tree_entries(&sha)?
        .into_iter()
        .filter(|e| e.path.ends_with(b".ts"))
        .collect();
    Ok(Inventory { sha, entries })
}

/// Усі звичайні й виконувані `blob` повного дерева коміту.
fn tree_entries(sha: &str) -> Result<Vec<Entry>, String> {
    let mut cmd = git();
    cmd.args(["ls-tree", "-r", "-z", "--full-tree", sha]);
    let listing = run(cmd, &format!("не вдалося прочитати дерево коміту {sha}"))?;
    parse_listing(&listing)
}

/// До яких умов gate входить запис коміту.
#[derive(Debug, Clone, Copy)]
pub struct Selection {
    pub check: bool,
    pub dup: bool,
}

/// Відбір для gate: записи, що входять хоча б в одну умову за політиками `CHECK` і `METRICS`,
/// застосованими до кореневого байтового ключа. Окремий від `inventory`, щоб звіти метрик
/// зберегли свій відбір.
pub struct GateInventory {
    pub entries: Vec<Entry>,
    pub selections: Vec<Selection>,
}

pub fn gate_inventory(rev: &str) -> Result<GateInventory, String> {
    let sha = resolve_commit(rev)?;
    let mut inv = GateInventory {
        entries: Vec::new(),
        selections: Vec::new(),
    };
    for entry in tree_entries(&sha)? {
        let selection = Selection {
            check: local_tree::CHECK.selects_key(&entry.path),
            dup: local_tree::METRICS.selects_key(&entry.path),
        };
        if selection.check || selection.dup {
            inv.entries.push(entry);
            inv.selections.push(selection);
        }
    }
    Ok(inv)
}

/// Розв'язує ревізію як окремий аргумент Git без переписування виразу. Неоднозначні імена
/// відхиляються: Git лише попереджає про них і обирає одну з цілей, а `-c` перекриває
/// користувацьке вимкнення попередження. Неоднозначність розпізнається за словом `ambiguous` у
/// `stderr` Git.
fn resolve_commit(rev: &str) -> Result<String, String> {
    let fail = |msg: &str| format!("не вдалося розв'язати коміт {rev}: {msg}");
    let out = git()
        .args([
            "-c",
            "core.warnAmbiguousRefs=true",
            "rev-parse",
            "--verify",
            "--end-of-options",
        ])
        .arg(rev)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("не вдалося запустити git: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(fail(stderr.trim()));
    }
    if stderr.contains("ambiguous") {
        return Err(fail(&format!("ревізія неоднозначна ({})", stderr.trim())));
    }
    let object = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if object.is_empty() {
        return Err(fail("порожня відповідь"));
    }
    // Розіменування за вже зафіксованим ідентифікатором: тег -> коміт, інші типи відхиляються.
    let mut cmd = git();
    cmd.args(["rev-parse", "--verify", "--end-of-options"])
        .arg(format!("{object}^{{commit}}"));
    let out = run(cmd, &format!("не вдалося розв'язати коміт {rev}"))?;
    let sha = String::from_utf8_lossy(&out).trim().to_string();
    if sha.is_empty() {
        return Err(fail("порожня відповідь"));
    }
    Ok(sha)
}

/// Розбирає вивід `ls-tree -z`, лишаючи всі `blob` із режимами `100644` і `100755` (посилання й
/// gitlink пропускаються); вибір за іменем лишається споживачеві. Результат відсортований за
/// байтами шляху.
fn parse_listing(listing: &[u8]) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for record in listing.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let bad = || "неочікуваний формат переліку дерева".to_string();
        let tab = record.iter().position(|b| *b == b'\t').ok_or_else(bad)?;
        let (meta, path) = (&record[..tab], &record[tab + 1..]);
        let meta = std::str::from_utf8(meta).map_err(|_| bad())?;
        let mut parts = meta.split(' ');
        let (mode, kind, oid) = match (parts.next(), parts.next(), parts.next()) {
            (Some(m), Some(k), Some(o)) => (m, k, o),
            _ => return Err(bad()),
        };
        if kind == "blob" && matches!(mode, "100644" | "100755") {
            entries.push(Entry {
                path: path.to_vec(),
                oid: oid.to_string(),
            });
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// Читає вміст кожного запису за ідентифікатором його об'єкта; результат
/// відповідає порядку `entries`.
pub fn read_contents(entries: &[Entry]) -> Vec<Result<Vec<u8>, ReadError>> {
    let mut results: Vec<Result<Vec<u8>, ReadError>> = Vec::with_capacity(entries.len());
    read_batch(entries, &mut results);
    // Збій пакета обриває решту відповідей, тому непрочитані об'єкти перевіряємо поодинці.
    for entry in &entries[results.len()..] {
        results.push(read_single(&entry.oid));
    }
    results
}

fn read_single(oid: &str) -> Result<Vec<u8>, ReadError> {
    let mut cmd = git();
    cmd.args(["cat-file", "blob", oid]).stderr(Stdio::null());
    run(cmd, "").map_err(|_| ReadError)
}

/// Дописує до `results` відповіді пакета до першого збою протоколу чи процесу.
fn read_batch(entries: &[Entry], results: &mut Vec<Result<Vec<u8>, ReadError>>) {
    if entries.is_empty() {
        return;
    }
    let spawned = git()
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else { return };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };
    let request: String = entries.iter().map(|e| format!("{}\n", e.oid)).collect();
    // Запис у окремому потоці, щоб великі відповіді не блокувалися на заповненому каналі.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(request.as_bytes());
    });

    let mut reader = BufReader::new(stdout);
    for entry in entries {
        match read_one(&mut reader, &entry.oid) {
            Some(item) => results.push(item),
            None => break,
        }
    }
    drop(reader);
    let _ = child.kill();
    let _ = child.wait();
    let _ = writer.join();
}

/// `None` — потік обірвано або сформовано неочікувано; об'єкт слід перечитати окремо.
fn read_one(
    reader: &mut BufReader<std::process::ChildStdout>,
    oid: &str,
) -> Option<Result<Vec<u8>, ReadError>> {
    let mut header = String::new();
    if reader.read_line(&mut header).ok()? == 0 || !header.ends_with('\n') {
        return None;
    }
    let mut parts = header.trim_end_matches('\n').split(' ');
    if parts.next()? != oid {
        return None;
    }
    let kind = parts.next()?;
    if kind == "missing" {
        return Some(Err(ReadError));
    }
    let size: usize = parts.next()?.parse().ok()?;
    let mut content = Vec::new();
    reader
        .by_ref()
        .take(size as u64)
        .read_to_end(&mut content)
        .ok()?;
    let mut lf = [0u8; 1];
    reader.read_exact(&mut lf).ok()?;
    if content.len() != size || lf[0] != b'\n' {
        return None;
    }
    Some(if kind == "blob" {
        Ok(content)
    } else {
        Err(ReadError)
    })
}
