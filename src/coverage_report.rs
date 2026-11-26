//! Зіставлення шляхів `SF` зі звіту LCOV з повним інвентарем `.ts` і явними тестовими виключеннями.
//!
//! Усе зіставлення лексичне: файлів на диску ніхто не відкриває, тож ключі старого коміту працюють
//! без checkout.

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsStr;

use serde::Serialize;

use crate::complexity_report::{FileInput, Origin, Report, path_fields};
use crate::duplication_report::density_tenths;
use crate::lcov::{self, Counts, Defect};
use crate::ts_coverage;

type Comps = Vec<Vec<u8>>;

#[derive(Debug, PartialEq, Eq)]
pub enum Owner {
    /// Індекс основного файла в інвентарі.
    Main(usize),
    /// Блок цілком ігнорується: інший суфікс або тест.
    Ignored,
    /// `.ts`, якому не відповідає жоден файл інвентарю.
    Unresolved,
}

pub struct Matcher {
    rules: PathRules,
    root: Comps,
    /// Скільки перших компонентів `root` — префікс диска чи ресурсу.
    root_floor: usize,
    index: HashMap<Vec<u8>, usize>,
    tests: Vec<Vec<u8>>,
}

/// Правила розбору шляху. Нативний виклик обирає правила цільової ОС; юніт-тести застосовують
/// Windows-правила до синтетичних шляхів на будь-якому хості.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathRules {
    /// Роздільник лише `/`; `\` і `:` — звичайні байти імені.
    Posix,
    /// Роздільники `/` і `\`, диск, UNC-ресурс та розширений префікс `\\?\`.
    Windows,
}

impl PathRules {
    pub const NATIVE: PathRules = if cfg!(windows) {
        PathRules::Windows
    } else {
        PathRules::Posix
    };

    fn is_sep(self, b: u8) -> bool {
        b == b'/' || (self == PathRules::Windows && b == b'\\')
    }

    fn split(self, path: &[u8]) -> impl Iterator<Item = &[u8]> {
        path.split(move |b| self.is_sep(*b))
            .filter(|c| !c.is_empty())
    }
}

/// Результат розбору шляху за правилами.
#[derive(Debug, PartialEq, Eq)]
enum Parsed<'a> {
    Relative(&'a [u8]),
    /// `prefix` — ідентичність диска чи UNC-ресурсу; `None` для POSIX і шляху від кореня без диска.
    Absolute {
        prefix: Option<Vec<u8>>,
        rest: &'a [u8],
    },
    /// Форма без лексичного змісту: диск без кореня (`C:a`), неповний UNC, невідомий префікс.
    Unsupported,
}

fn drive_at(p: &[u8]) -> Option<u8> {
    (p.len() >= 2 && p[0].is_ascii_alphabetic() && p[1] == b':').then(|| p[0].to_ascii_uppercase())
}

fn parse<'a>(rules: PathRules, p: &'a [u8]) -> Parsed<'a> {
    if rules == PathRules::Posix {
        return if p.first() == Some(&b'/') {
            Parsed::Absolute {
                prefix: None,
                rest: p,
            }
        } else {
            Parsed::Relative(p)
        };
    }
    let sep = |i: usize| p.get(i).is_some_and(|b| rules.is_sep(*b));
    let drive = |p: &'a [u8]| match drive_at(p) {
        Some(letter) if p.len() == 2 || rules.is_sep(p[2]) => Some(Parsed::Absolute {
            prefix: Some(vec![letter, b':']),
            rest: &p[2..],
        }),
        Some(_) => Some(Parsed::Unsupported),
        None => None,
    };
    let unc = |p: &'a [u8]| {
        let mut parts = p.splitn(3, |b| rules.is_sep(*b));
        let (server, share) = (parts.next()?, parts.next()?);
        if server.is_empty() || share.is_empty() {
            return None;
        }
        let mut id = b"\\\\".to_vec();
        id.extend(server.to_ascii_lowercase());
        id.push(b'\\');
        id.extend(share.to_ascii_lowercase());
        Some(Parsed::Absolute {
            prefix: Some(id),
            rest: parts.next().unwrap_or(b""),
        })
    };
    if sep(0) && sep(1) {
        // `\\?\` і `\\.\`: розширена форма; `\\сервер\ресурс`: звичайний UNC.
        if matches!(p.get(2), Some(b'?' | b'.')) && sep(3) {
            let rest = &p[4..];
            if rest.len() >= 4 && rest[..3].eq_ignore_ascii_case(b"UNC") && rules.is_sep(rest[3]) {
                return unc(&rest[4..]).unwrap_or(Parsed::Unsupported);
            }
            return drive(rest).unwrap_or(Parsed::Unsupported);
        }
        return unc(&p[2..]).unwrap_or(Parsed::Unsupported);
    }
    if sep(0) {
        return Parsed::Absolute {
            prefix: None,
            rest: p,
        };
    }
    drive(p).unwrap_or(Parsed::Relative(p))
}

/// Лексично дописує `rel` до `base`, враховуючи `.` і `..`. `..` не знімає перших `floor`
/// компонентів (префікс диска чи ресурсу). Без `keep_up` вихід над корінь ігнорується, з ним —
/// лишається ведучими `..`.
fn normalize(rules: PathRules, base: &[Vec<u8>], floor: usize, rel: &[u8], keep_up: bool) -> Comps {
    let mut out: Comps = base.to_vec();
    for part in rules.split(rel) {
        match part {
            b"." => {}
            b".." => match out.last() {
                Some(last) if out.len() > floor && last.as_slice() != b".." => {
                    out.pop();
                }
                _ if keep_up => out.push(part.to_vec()),
                _ => {}
            },
            _ => out.push(part.to_vec()),
        }
    }
    out
}

/// Абсолютні компоненти шляху (першим іде префікс, якщо він є) і кількість захищених компонентів.
/// Відносний шлях дає порожню базу.
fn absolute(rules: PathRules, p: &[u8]) -> Option<(Comps, usize)> {
    match parse(rules, p) {
        Parsed::Relative(rest) => Some((normalize(rules, &[], 0, rest, false), 0)),
        Parsed::Absolute { prefix, rest } => {
            let base: Comps = prefix.into_iter().collect();
            let floor = base.len();
            Some((normalize(rules, &base, floor, rest, false), floor))
        }
        Parsed::Unsupported => None,
    }
}

fn os_bytes(s: &OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        std::os::unix::ffi::OsStrExt::as_bytes(s).to_vec()
    }
    #[cfg(not(unix))]
    {
        s.to_string_lossy().into_owned().into_bytes()
    }
}

/// Перевіряє шляхи `--tests`: відносні, усередині кореня. Існування на диску не потрібне.
pub fn parse_tests(args: &[&OsStr]) -> Result<Vec<Vec<u8>>, String> {
    parse_tests_with(PathRules::NATIVE, args)
}

pub fn parse_tests_with(rules: PathRules, args: &[&OsStr]) -> Result<Vec<Vec<u8>>, String> {
    args.iter()
        .map(|arg| {
            let bytes = os_bytes(arg);
            if bytes.is_empty() {
                return Err("--tests: порожній шлях".to_string());
            }
            let Parsed::Relative(rest) = parse(rules, &bytes) else {
                return Err(format!(
                    "--tests: шлях має бути відносним: {}",
                    arg.to_string_lossy()
                ));
            };
            let comps = normalize(rules, &[], 0, rest, true);
            if comps.first().is_some_and(|c| c.as_slice() == b"..") {
                return Err(format!(
                    "--tests: шлях виходить за корінь: {}",
                    arg.to_string_lossy()
                ));
            }
            Ok(comps.join(&b'/'))
        })
        .collect()
}

impl Matcher {
    /// `root` — абсолютна канонічна тека, `keys` — повний інвентар у його порядку, `tests` —
    /// результат `parse_tests`.
    pub fn with_rules<'a>(
        rules: PathRules,
        root: &[u8],
        keys: impl IntoIterator<Item = &'a [u8]>,
        tests: Vec<Vec<u8>>,
    ) -> Self {
        let (root_comps, root_floor) = absolute(rules, root).unwrap_or_default();
        let index = keys
            .into_iter()
            .enumerate()
            .map(|(i, k)| (k.to_vec(), i))
            .collect();
        Matcher {
            rules,
            root: root_comps,
            root_floor,
            index,
            tests,
        }
    }

    fn is_test(&self, key: &[u8]) -> bool {
        self.tests.iter().any(|t| {
            t.is_empty() || key == t.as_slice() || (key.starts_with(t) && key[t.len()] == b'/')
        })
    }

    /// Ключі-кандидати під коренем у порядку пошуку. `report_dir` — абсолютна канонічна тека звіту.
    fn candidates(&self, report_dir: &[u8], sf: &[u8]) -> Vec<Vec<u8>> {
        let rules = self.rules;
        let under_root = |full: Comps| {
            full.starts_with(&self.root)
                .then(|| full[self.root.len()..].join(&b'/'))
        };
        match parse(rules, sf) {
            Parsed::Unsupported => return Vec::new(),
            Parsed::Absolute { prefix, rest } => {
                let base: Comps = prefix.into_iter().collect();
                let floor = base.len();
                return under_root(normalize(rules, &base, floor, rest, false))
                    .into_iter()
                    .collect();
            }
            Parsed::Relative(_) => {}
        }
        let Some((dir, _)) = absolute(rules, report_dir) else {
            return Vec::new();
        };
        let base_floor = self.root_floor;
        let bases: Vec<Comps> = if dir.starts_with(&self.root) {
            (self.root.len()..=dir.len())
                .rev()
                .map(|n| dir[..n].to_vec())
                .collect()
        } else {
            vec![self.root.clone()]
        };
        bases
            .iter()
            .filter_map(|base| under_root(normalize(rules, base, base_floor, sf, false)))
            .collect()
    }

    /// Визначає, кому належить блок LCOV із шляхом `sf`, лексично й без доступу до диска.
    ///
    /// Збіг ключа інвентарю дає `Main`, або `Ignored`, якщо ключ є тестом. Без збігу блок
    /// ігнорується, коли `sf` не закінчується на `.ts` або серед кандидатів є тест; інакше це
    /// `Unresolved`.
    pub fn resolve(&self, report_dir: &[u8], sf: &[u8]) -> Owner {
        let candidates = self.candidates(report_dir, sf);
        if let Some((key, idx)) = candidates
            .iter()
            .find_map(|key| self.index.get(key).map(|idx| (key, *idx)))
        {
            return if self.is_test(key) {
                Owner::Ignored
            } else {
                Owner::Main(idx)
            };
        }
        let ts = normalize(self.rules, &[], 0, sf, true)
            .last()
            .is_some_and(|name| name.ends_with(b".ts"));
        if !ts || candidates.iter().any(|key| self.is_test(key)) {
            Owner::Ignored
        } else {
            Owner::Unresolved
        }
    }
}

/// Звіт LCOV з диска: байти й абсолютна канонічна тека, у якій він лежить.
pub struct LcovInput {
    pub dir: Vec<u8>,
    pub bytes: Vec<u8>,
}

#[derive(Serialize)]
struct Methodology {
    id: &'static str,
    version: u32,
}

#[derive(Serialize)]
struct Scope {
    input: &'static str,
    entries: &'static str,
    suffix: &'static str,
}

#[derive(Serialize)]
struct Metrics {
    #[serde(rename = "linesToCover")]
    lines_to_cover: Option<u64>,
    #[serde(rename = "uncoveredLines")]
    uncovered_lines: Option<u64>,
    #[serde(rename = "conditionsToCover")]
    conditions_to_cover: Option<u64>,
    #[serde(rename = "uncoveredConditions")]
    uncovered_conditions: Option<u64>,
    #[serde(rename = "lineCoverage")]
    line_coverage: Option<f64>,
    #[serde(rename = "branchCoverage")]
    branch_coverage: Option<f64>,
    coverage: Option<f64>,
}

#[derive(Serialize)]
struct FileRecord {
    path: Option<String>,
    #[serde(rename = "pathBytes", skip_serializing_if = "Option::is_none")]
    path_bytes: Option<String>,
    #[serde(rename = "inReport")]
    in_report: bool,
    #[serde(flatten)]
    metrics: Metrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
}

#[derive(Serialize)]
struct PathRecord {
    path: Option<String>,
    #[serde(rename = "pathBytes", skip_serializing_if = "Option::is_none")]
    path_bytes: Option<String>,
}

#[derive(Serialize)]
struct Document<'a> {
    sha: Option<&'a str>,
    source: &'static str,
    methodology: Methodology,
    scope: Scope,
    complete: bool,
    files: Vec<FileRecord>,
    unresolved: Vec<PathRecord>,
    total: Option<Metrics>,
}

/// Відсоток із десятими; нульовий знаменник дає `null`.
fn percent(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| density_tenths(part, whole) as f64 / 10.0)
}

fn metrics(c: Counts) -> Metrics {
    let covered_lines = c.lines - c.uncovered_lines;
    let covered_conditions = c.conditions - c.uncovered_conditions;
    Metrics {
        lines_to_cover: Some(c.lines),
        uncovered_lines: Some(c.uncovered_lines),
        conditions_to_cover: Some(c.conditions),
        uncovered_conditions: Some(c.uncovered_conditions),
        line_coverage: percent(covered_lines, c.lines),
        branch_coverage: percent(covered_conditions, c.conditions),
        coverage: percent(covered_lines + covered_conditions, c.lines + c.conditions),
    }
}

fn null_metrics() -> Metrics {
    Metrics {
        lines_to_cover: None,
        uncovered_lines: None,
        conditions_to_cover: None,
        uncovered_conditions: None,
        line_coverage: None,
        branch_coverage: None,
        coverage: None,
    }
}

fn file_counts(file: &FileInput, blocks: &[&lcov::Block]) -> Result<Counts, &'static str> {
    let content = file.content.as_ref().map_err(|e| e.reason())?;
    let lines = ts_coverage::analyze(&file.path, content).map_err(|e| e.reason())?;
    if blocks.is_empty() {
        return Ok(Counts {
            lines: lines.statement_lines,
            uncovered_lines: lines.statement_lines,
            conditions: 0,
            uncovered_conditions: 0,
        });
    }
    lcov::merge(
        blocks,
        usize::try_from(lines.physical).unwrap_or(usize::MAX),
    )
    .map_err(|d| match d {
        Defect::InvalidRecord => "lcov-invalid-record",
        Defect::LineOutOfRange => "lcov-line-out-of-range",
    })
}

/// Будує звіт покриття. `root` — абсолютна канонічна база зіставлення, `files` — повний інвентар,
/// `tests` — результат `parse_tests`.
pub fn build(
    origin: Origin<'_>,
    root: &[u8],
    files: Vec<FileInput>,
    reports: &[LcovInput],
    tests: Vec<Vec<u8>>,
) -> Report {
    build_with(PathRules::NATIVE, origin, root, files, reports, tests)
}

pub fn build_with(
    rules: PathRules,
    origin: Origin<'_>,
    root: &[u8],
    files: Vec<FileInput>,
    reports: &[LcovInput],
    tests: Vec<Vec<u8>>,
) -> Report {
    let matcher = Matcher::with_rules(rules, root, files.iter().map(|f| f.path.as_slice()), tests);
    let parsed: Vec<lcov::Report> = reports.iter().map(|r| lcov::parse(&r.bytes)).collect();
    let mut orphan = false;
    let mut groups: Vec<Vec<&lcov::Block>> = vec![Vec::new(); files.len()];
    let mut unresolved: BTreeSet<&[u8]> = BTreeSet::new();
    for (input, report) in reports.iter().zip(&parsed) {
        orphan |= report.orphan;
        for block in &report.blocks {
            match matcher.resolve(&input.dir, &block.path) {
                Owner::Main(i) => groups[i].push(block),
                Owner::Ignored => {}
                Owner::Unresolved => {
                    unresolved.insert(&block.path);
                }
            }
        }
    }

    let mut records = Vec::with_capacity(files.len());
    let mut sum = Counts::default();
    let mut file_error = false;
    for (file, blocks) in files.iter().zip(&groups) {
        // Тести лишаються в інвентарі для зіставлення, але не потрапляють у звіт.
        if matcher.is_test(&file.path) {
            continue;
        }
        let (path, path_bytes) = path_fields(&file.path);
        let in_report = !blocks.is_empty();
        match file_counts(file, blocks) {
            Ok(c) => {
                sum.lines += c.lines;
                sum.uncovered_lines += c.uncovered_lines;
                sum.conditions += c.conditions;
                sum.uncovered_conditions += c.uncovered_conditions;
                records.push(FileRecord {
                    path,
                    path_bytes,
                    in_report,
                    metrics: metrics(c),
                    error: None,
                });
            }
            Err(reason) => {
                file_error = true;
                records.push(FileRecord {
                    path,
                    path_bytes,
                    in_report,
                    metrics: null_metrics(),
                    error: Some(reason),
                });
            }
        }
    }
    let complete = !file_error && unresolved.is_empty() && !orphan;
    let (sha, input) = origin.fields();
    let doc = Document {
        sha,
        source: "osavul",
        methodology: Methodology {
            id: "ts-coverage",
            version: 1,
        },
        scope: Scope {
            input,
            entries: "regular-files",
            suffix: ".ts",
        },
        complete,
        files: records,
        unresolved: unresolved
            .into_iter()
            .map(|p| {
                let (path, path_bytes) = path_fields(p);
                PathRecord { path, path_bytes }
            })
            .collect(),
        total: complete.then(|| metrics(sum)),
    };
    let mut json =
        serde_json::to_vec(&doc).expect("серіалізація звіту не може завершитись помилкою");
    json.push(b'\n');
    Report { json, complete }
}

/// Читає запитані LCOV з диска; шлях відраховується від поточної теки процесу, тека звіту
/// канонізується.
fn read_reports(paths: &[&OsStr]) -> Result<Vec<LcovInput>, String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("не вдалося визначити поточну теку: {e}"))?;
    paths
        .iter()
        .map(|p| {
            let shown = p.to_string_lossy();
            let full = cwd.join(p);
            let bytes = std::fs::read(&full)
                .map_err(|e| format!("не вдалося прочитати LCOV {shown}: {e}"))?;
            let dir = full
                .parent()
                .and_then(|d| std::fs::canonicalize(d).ok())
                .ok_or_else(|| format!("не вдалося визначити теку LCOV {shown}"))?;
            Ok(LcovInput {
                dir: os_bytes(dir.as_os_str()),
                bytes,
            })
        })
        .collect()
}

fn canonical_root(root: &std::path::Path) -> Result<Vec<u8>, String> {
    std::fs::canonicalize(root)
        .map(|p| os_bytes(p.as_os_str()))
        .map_err(|e| format!("не вдалося визначити корінь {}: {e}", root.display()))
}

/// Звіт для коміту `rev`; `tests` — результат `parse_tests`. `Err` — фатальна відмова до
/// публікації.
pub fn measure(rev: &str, lcov: &[&OsStr], tests: Vec<Vec<u8>>) -> Result<Report, String> {
    let (sha, files) = crate::complexity_report::git_inputs(rev)?;
    let top = crate::git_tree::match_root()?;
    let root = canonical_root(std::path::Path::new(&os_string(&top)))?;
    let reports = read_reports(lcov)?;
    Ok(build(Origin::Commit(&sha), &root, files, &reports, tests))
}

/// Звіт для локальної теки `path`; файл замість теки — фатальна помилка.
pub fn measure_local(
    path: &std::path::Path,
    lcov: &[&OsStr],
    tests: Vec<Vec<u8>>,
) -> Result<Report, String> {
    let (kind, files) = crate::complexity_report::local_inputs(path)?;
    if kind != crate::local_tree::InputKind::Directory {
        return Err(format!("--path має бути текою: {}", path.display()));
    }
    let root = canonical_root(path)?;
    let reports = read_reports(lcov)?;
    Ok(build(Origin::Directory, &root, files, &reports, tests))
}

#[cfg(unix)]
fn os_string(bytes: &[u8]) -> std::ffi::OsString {
    std::os::unix::ffi::OsStringExt::from_vec(bytes.to_vec())
}

#[cfg(not(unix))]
fn os_string(bytes: &[u8]) -> std::ffi::OsString {
    String::from_utf8_lossy(bytes).into_owned().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lcov;
    use std::path::Path;

    const ROOT: &str = "/work/corpus";
    const DEFAULT_INVENTORY: &str = "src/main.ts,src/fallback.ts,src/broken.ts,tests/example.ts";

    /// Один набір входів зіставлення: правила й форми кореня, теки звіту та абсолютного `SF`.
    struct Form {
        rules: PathRules,
        /// Корінь, як його повертає канонізація.
        root: &'static str,
        /// Підстановка `{root}` у фрагментах LCOV.
        sf_root: &'static str,
        /// Тека звіту поза коренем.
        outside: &'static str,
    }

    const POSIX: Form = Form {
        rules: PathRules::Posix,
        root: ROOT,
        sf_root: ROOT,
        outside: "/elsewhere/reports",
    };
    const WINDOWS_PLAIN: Form = Form {
        rules: PathRules::Windows,
        root: "C:\\work\\corpus",
        sf_root: "C:/work/corpus",
        outside: "D:\\elsewhere\\reports",
    };
    // Канонізація Windows дає розширену форму, а LCOV міг записати звичайну іншого регістру.
    const WINDOWS_VERBATIM: Form = Form {
        rules: PathRules::Windows,
        root: "\\\\?\\C:\\work\\corpus",
        sf_root: "c:\\work\\corpus",
        outside: "\\\\?\\UNC\\srv\\share\\reports",
    };
    const FORMS: [&Form; 3] = [&POSIX, &WINDOWS_PLAIN, &WINDOWS_VERBATIM];

    fn tests_of(rules: PathRules, tests: &str) -> Vec<Vec<u8>> {
        let tests: Vec<&OsStr> = tests
            .split(',')
            .filter(|t| *t != "-")
            .map(OsStr::new)
            .collect();
        parse_tests_with(rules, &tests).unwrap()
    }

    fn matcher_in(form: &Form, inventory: &str, tests: &str) -> Matcher {
        Matcher::with_rules(
            form.rules,
            form.root.as_bytes(),
            inventory.split(',').map(str::as_bytes),
            tests_of(form.rules, tests),
        )
    }

    fn matcher(inventory: &str, tests: &str) -> Matcher {
        matcher_in(&POSIX, inventory, tests)
    }

    fn fragment(dir: &Path, file: &str, name: &str) -> String {
        let text = std::fs::read_to_string(dir.join(file)).unwrap();
        let marker = format!("#@ {name}");
        let mut lines = text.lines();
        assert!(lines.any(|l| l == marker), "немає фрагмента {file}#{name}");
        let mut out = String::new();
        for line in lines.take_while(|l| !l.starts_with("#@ ")) {
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    #[test]
    fn tests_arguments_must_be_relative_and_inside_root() {
        for rules in [PathRules::Posix, PathRules::Windows] {
            let ok =
                parse_tests_with(rules, &[OsStr::new("test/"), OsStr::new("./a/../b.ts")]).unwrap();
            assert_eq!(ok, vec![b"test".to_vec(), b"b.ts".to_vec()], "{rules:?}");
            for bad in ["/abs", "..", "a/../../b", ""] {
                assert!(
                    parse_tests_with(rules, &[OsStr::new(bad)]).is_err(),
                    "{rules:?} {bad}"
                );
            }
        }
    }

    // Виявляє приймання `--tests`, що за Windows-правилами не є відносним усередині кореня.
    #[test]
    fn windows_tests_arguments_reject_every_absolute_form() {
        let w = PathRules::Windows;
        let ok = parse_tests_with(w, &[OsStr::new("a\\b\\..\\c.ts")]).unwrap();
        assert_eq!(ok, vec![b"a/c.ts".to_vec()]);
        for bad in [
            "C:\\x",
            "C:/x",
            "C:x",
            "\\abs",
            "\\\\srv\\share\\x",
            "\\\\?\\C:\\x",
            "a\\..\\..\\b",
        ] {
            assert!(parse_tests_with(w, &[OsStr::new(bad)]).is_err(), "{bad}");
        }
        // У POSIX зворотна скісна риска й двокрапка належать імені.
        let p = parse_tests_with(PathRules::Posix, &[OsStr::new("C:\\x")]).unwrap();
        assert_eq!(p, vec![b"C:\\x".to_vec()]);
    }

    fn resolve_in(form: &Form, report_dir: &str, sf: &str, inventory: &str) -> Owner {
        matcher_in(form, inventory, "-").resolve(report_dir.as_bytes(), sf.as_bytes())
    }

    // Виявляє змішування дисків і ресурсів, префікса за текстом і форм кореня.
    #[test]
    fn windows_absolute_sf_matches_only_inside_the_same_root() {
        let w = PathRules::Windows;
        let inv = "src/a.ts";
        for root in [
            "C:\\work\\corpus",
            "c:/WORK/corpus".replace("WORK", "work").as_str(),
            "\\\\?\\C:\\work\\corpus",
            "\\\\.\\C:\\work\\corpus",
        ] {
            let m = Matcher::with_rules(w, root.as_bytes(), [inv.as_bytes()], vec![]);
            for sf in [
                "C:\\work\\corpus\\src\\a.ts",
                "c:/work/corpus/src/./x/../a.ts",
                "\\\\?\\C:\\work\\corpus\\src\\a.ts",
                "//?/c:/work/corpus/src/a.ts",
            ] {
                assert_eq!(
                    m.resolve(root.as_bytes(), sf.as_bytes()),
                    Owner::Main(0),
                    "{root} {sf}"
                );
            }
            for sf in [
                "D:\\work\\corpus\\src\\a.ts",
                "C:\\work\\corpus2\\src\\a.ts",
                "C:\\work\\Corpus\\src\\a.ts",
                "C:\\work\\corpus\\src\\A.ts",
                "C:\\work\\src\\a.ts",
                "C:\\work\\corpus\\..\\x\\src\\a.ts",
                "\\work\\corpus\\src\\a.ts",
                "\\\\srv\\share\\work\\corpus\\src\\a.ts",
                "C:src\\a.ts",
            ] {
                assert_eq!(
                    m.resolve(root.as_bytes(), sf.as_bytes()),
                    Owner::Unresolved,
                    "{root} {sf}"
                );
            }
        }
    }

    #[test]
    fn windows_unc_root_distinguishes_share_and_accepts_extended_form() {
        let w = PathRules::Windows;
        let m = Matcher::with_rules(
            w,
            b"\\\\?\\UNC\\Srv\\Share\\proj",
            [b"a.ts".as_slice()],
            vec![],
        );
        let dir = b"\\\\?\\UNC\\Srv\\Share\\proj\\reports";
        for sf in [
            "\\\\srv\\share\\proj\\a.ts",
            "//SRV/SHARE/proj/a.ts",
            "\\\\?\\UNC\\srv\\share\\proj\\a.ts",
            "..\\a.ts",
            "a.ts",
        ] {
            assert_eq!(m.resolve(dir, sf.as_bytes()), Owner::Main(0), "{sf}");
        }
        for sf in [
            "\\\\srv\\other\\proj\\a.ts",
            "\\\\other\\share\\proj\\a.ts",
            "\\\\srv\\share\\proj2\\a.ts",
            "\\\\srv\\share\\a.ts",
            "C:\\proj\\a.ts",
        ] {
            assert_eq!(m.resolve(dir, sf.as_bytes()), Owner::Unresolved, "{sf}");
        }
    }

    // `..` у відносному SF не виходить за префікс диска чи ресурсу.
    #[test]
    fn windows_relative_sf_search_goes_up_to_root_and_never_past_the_prefix() {
        let f = &WINDOWS_PLAIN;
        let inv = "src/a.ts,pkg/src/a.ts";
        assert_eq!(
            resolve_in(f, "C:\\work\\corpus\\pkg\\r", "src\\a.ts", inv),
            Owner::Main(1)
        );
        assert_eq!(
            resolve_in(f, "C:/work/corpus/pkg/r", "..\\..\\src\\a.ts", inv),
            Owner::Main(0)
        );
        assert_eq!(
            resolve_in(
                f,
                "C:\\work\\corpus",
                "..\\..\\..\\..\\work\\corpus\\src\\a.ts",
                inv
            ),
            Owner::Main(0)
        );
        // Тека звіту на іншому диску бачить лише корінь.
        assert_eq!(resolve_in(f, "D:\\r", "src\\a.ts", inv), Owner::Main(0));
        assert_eq!(
            resolve_in(f, "D:\\r", "..\\src\\a.ts", inv),
            Owner::Unresolved
        );
        // Нерозв'язаний SF зберігає вихідні байти, а не нормалізований ключ.
        let lcov = lcov_in(
            "C:\\work\\corpus",
            "SF:src\\nope.ts\nDA:1,1\nend_of_record\n",
        );
        let report = build_with(
            PathRules::Windows,
            Origin::Directory,
            f.root.as_bytes(),
            vec![file(b"src/a.ts", b"const a = 1;\n")],
            &[lcov],
            vec![],
        );
        assert_eq!(json_of(&report)["unresolved"][0]["path"], "src\\nope.ts");
    }

    // Виявляє зіставлення тестової теки за префіксом рядка замість цілого компонента шляху.
    #[test]
    fn tests_match_whole_components_only() {
        let m = matcher("test/a.ts,testing/a.ts,test.ts", "test");
        assert_eq!(m.resolve(ROOT.as_bytes(), b"test/a.ts"), Owner::Ignored);
        assert_eq!(m.resolve(ROOT.as_bytes(), b"testing/a.ts"), Owner::Main(1));
        assert_eq!(m.resolve(ROOT.as_bytes(), b"test.ts"), Owner::Main(2));
    }

    #[test]
    fn non_ts_is_ignored_even_without_inventory_match() {
        let m = matcher("a.ts", "-");
        assert_eq!(m.resolve(ROOT.as_bytes(), b"a.js"), Owner::Ignored);
        assert_eq!(m.resolve(ROOT.as_bytes(), b"a.tsx"), Owner::Ignored);
        assert_eq!(m.resolve(ROOT.as_bytes(), b"b.ts"), Owner::Unresolved);
    }

    #[test]
    fn corpus_paths_match_expected_tsv() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts-coverage");
        let table = std::fs::read_to_string(dir.join("expected.tsv")).unwrap();
        let mut rows = 0;
        for row in table.lines().skip(1) {
            let f: Vec<&str> = row.split('\t').collect();
            let Some(input) = f[2].strip_prefix('@') else {
                continue;
            };
            let (file, name) = input.split_once('#').unwrap();
            for form in FORMS {
                let text = fragment(&dir, file, name).replace("{root}", form.sf_root);
                let report = lcov::parse(text.as_bytes());
                assert_eq!(report.blocks.len(), 1, "{}", f[0]);
                let report_dir = match f[3] {
                    "-" => format!(
                        "{}/{}",
                        form.root,
                        Path::new(file).parent().unwrap().display()
                    ),
                    "!outside" => form.outside.to_string(),
                    place => format!("{}/{place}", form.root),
                };
                let inventory = if f[6] == "-" { DEFAULT_INVENTORY } else { f[6] };
                let m = matcher_in(form, inventory, f[4]);
                let keys: Vec<&str> = inventory.split(',').collect();
                let got = match m.resolve(report_dir.as_bytes(), &report.blocks[0].path) {
                    Owner::Main(i) => format!("main:{}", keys[i]),
                    Owner::Ignored => "ignored".to_string(),
                    Owner::Unresolved => "unresolved".to_string(),
                };
                assert_eq!(got, f[5], "{} {:?} {}", f[0], form.rules, form.root);
            }
            rows += 1;
        }
        assert_eq!(rows, 20);
    }

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn file(path: &[u8], content: &[u8]) -> FileInput {
        FileInput {
            path: path.to_vec(),
            content: Ok(content.to_vec()),
        }
    }

    fn lcov_in(dir: &str, text: &str) -> LcovInput {
        LcovInput {
            dir: dir.as_bytes().to_vec(),
            bytes: text.as_bytes().to_vec(),
        }
    }

    fn json_of(report: &Report) -> serde_json::Value {
        serde_json::from_slice(&report.json).unwrap()
    }

    // Виявляє зміну точних байтів JSON: порядку полів, округлення відсотків чи `null` без гілок.
    #[test]
    fn exact_json_bytes_and_percent_formats() {
        let files = vec![
            file(b"a.ts", b"a();\nb();\nc();\n"),
            file(b"b.ts", b"a();"),
            file(b"c.ts", b"a();"),
        ];
        let reports = [lcov_in(
            ROOT,
            "SF:a.ts\nDA:1,1\nDA:2,0\nDA:3,0\nend_of_record\nSF:c.ts\nDA:1,5\nend_of_record\n",
        )];
        let report = build(
            Origin::Commit(SHA),
            ROOT.as_bytes(),
            files,
            &reports,
            vec![],
        );
        assert!(report.complete);
        let zero = "\"conditionsToCover\":0,\"uncoveredConditions\":0";
        let want = format!(
            "{{\"sha\":\"{SHA}\",\"source\":\"osavul\",\"methodology\":{{\"id\":\"ts-coverage\",\"version\":1}},\"scope\":{{\"input\":\"git-tree\",\"entries\":\"regular-files\",\"suffix\":\".ts\"}},\"complete\":true,\"files\":[\
{{\"path\":\"a.ts\",\"inReport\":true,\"linesToCover\":3,\"uncoveredLines\":2,{zero},\"lineCoverage\":33.3,\"branchCoverage\":null,\"coverage\":33.3}},\
{{\"path\":\"b.ts\",\"inReport\":false,\"linesToCover\":1,\"uncoveredLines\":1,{zero},\"lineCoverage\":0.0,\"branchCoverage\":null,\"coverage\":0.0}},\
{{\"path\":\"c.ts\",\"inReport\":true,\"linesToCover\":1,\"uncoveredLines\":0,{zero},\"lineCoverage\":100.0,\"branchCoverage\":null,\"coverage\":100.0}}],\
\"unresolved\":[],\"total\":{{\"linesToCover\":5,\"uncoveredLines\":3,{zero},\"lineCoverage\":40.0,\"branchCoverage\":null,\"coverage\":40.0}}}}\n"
        );
        assert_eq!(String::from_utf8(report.json).unwrap(), want);
    }

    #[test]
    fn empty_inventory_is_complete_with_zero_totals() {
        let report = build(Origin::Directory, ROOT.as_bytes(), vec![], &[], vec![]);
        assert!(report.complete);
        let want = "{\"sha\":null,\"source\":\"osavul\",\"methodology\":{\"id\":\"ts-coverage\",\"version\":1},\"scope\":{\"input\":\"directory\",\"entries\":\"regular-files\",\"suffix\":\".ts\"},\"complete\":true,\"files\":[],\"unresolved\":[],\"total\":{\"linesToCover\":0,\"uncoveredLines\":0,\"conditionsToCover\":0,\"uncoveredConditions\":0,\"lineCoverage\":null,\"branchCoverage\":null,\"coverage\":null}}\n";
        assert_eq!(String::from_utf8(report.json).unwrap(), want);
    }

    // Виявляє втрату шляхів не в UTF-8 у `files` і `unresolved` та хибний повний звіт при них.
    #[test]
    fn non_utf8_paths_use_path_bytes_in_files_and_unresolved() {
        let files = vec![file(b"\xff.ts", b"a();")];
        let reports = [LcovInput {
            dir: ROOT.as_bytes().to_vec(),
            bytes: b"SF:\xfe.ts\nend_of_record\nSF:b.ts\nSF:b.ts\nDA:1,1\nend_of_record\n".to_vec(),
        }];
        let report = build(Origin::Directory, ROOT.as_bytes(), files, &reports, vec![]);
        assert!(!report.complete);
        let text = String::from_utf8(report.json).unwrap();
        assert!(
            text.contains(
                "\"files\":[{\"path\":null,\"pathBytes\":\"ff2e7473\",\"inReport\":false,"
            )
        );
        assert!(text.ends_with(
            "\"unresolved\":[{\"path\":\"b.ts\"},{\"path\":null,\"pathBytes\":\"fe2e7473\"}],\"total\":null}\n"
        ));
    }

    // Виявляє недетермінізм: байти звіту не мають залежати від порядку файлів LCOV і блоків у них.
    #[test]
    fn report_and_block_order_do_not_change_bytes() {
        let main = std::fs::read(corpus().join("src/main.ts")).unwrap();
        let one = fragment(&corpus(), "reports/merge-a.info", "repeat");
        let (first, second) = one.split_once("end_of_record\n").unwrap();
        let swapped = format!("{second}{first}end_of_record\n");
        let run = |reports: &[LcovInput]| {
            build(
                Origin::Directory,
                ROOT.as_bytes(),
                vec![file(b"src/main.ts", &main)],
                reports,
                vec![],
            )
            .json
        };
        let x = fragment(&corpus(), "reports/merge-a.info", "base");
        let y = fragment(&corpus(), "reports/merge-b.info", "overlap");
        let dir = format!("{ROOT}/reports");
        assert_eq!(
            run(&[lcov_in(&dir, &x), lcov_in(&dir, &y)]),
            run(&[lcov_in(&dir, &y), lcov_in(&dir, &x)])
        );
        assert_eq!(run(&[lcov_in(&dir, &one)]), run(&[lcov_in(&dir, &swapped)]));
    }

    fn corpus() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts-coverage")
    }

    fn cell<T: std::fmt::Display>(v: Option<T>) -> String {
        v.map_or("-".to_string(), |v| v.to_string())
    }

    fn num(v: &serde_json::Value) -> String {
        if v.is_null() {
            "-".to_string()
        } else if let Some(f) = v.as_f64().filter(|_| v.is_f64()) {
            format!("{f:.1}")
        } else {
            v.to_string()
        }
    }

    #[test]
    fn corpus_reports_match_expected_tsv() {
        let dir = corpus();
        let table = std::fs::read_to_string(dir.join("expected.tsv")).unwrap();
        let mut rows = 0;
        for row in table.lines().skip(1) {
            let f: Vec<&str> = row.split('\t').collect();
            let Some(inputs) = f[2].strip_prefix('=') else {
                continue;
            };
            for form in FORMS {
                let reports: Vec<LcovInput> = inputs
                    .split(',')
                    .filter(|i| *i != "-")
                    .map(|input| {
                        let (file, name) = input.split_once('#').unwrap();
                        let text = fragment(&dir, file, name).replace("{root}", form.sf_root);
                        let place = Path::new(file).parent().unwrap().display();
                        lcov_in(&format!("{}/{place}", form.root), &text)
                    })
                    .collect();
                let files: Vec<FileInput> = f[3]
                    .split(',')
                    .filter(|k| *k != "-")
                    .map(|key| match key.split_once('=') {
                        None => file(key.as_bytes(), &std::fs::read(dir.join(key)).unwrap()),
                        Some((k, "read")) => FileInput {
                            path: k.as_bytes().to_vec(),
                            content: Err(crate::git_tree::ReadError),
                        },
                        Some((k, "utf8")) => file(k.as_bytes(), b"\xff\xfe"),
                        Some((k, "empty")) => file(k.as_bytes(), b""),
                        Some(_) => panic!("{}: невідомий маркер {key}", f[0]),
                    })
                    .collect();
                let report = build_with(
                    form.rules,
                    Origin::Directory,
                    form.root.as_bytes(),
                    files,
                    &reports,
                    tests_of(form.rules, f[4]),
                );
                let v = json_of(&report);
                assert_eq!(report.complete, f[5] == "complete", "{}: complete", f[0]);
                assert_eq!(v["complete"], report.complete, "{}", f[0]);
                let got: Vec<String> = v["files"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| {
                        let cols = [
                            r["path"].as_str().unwrap().to_string(),
                            r["inReport"].to_string(),
                            num(&r["linesToCover"]),
                            num(&r["uncoveredLines"]),
                            num(&r["conditionsToCover"]),
                            num(&r["uncoveredConditions"]),
                            num(&r["lineCoverage"]),
                            num(&r["branchCoverage"]),
                            num(&r["coverage"]),
                            cell(r["error"].as_str()),
                        ];
                        cols.join("|")
                    })
                    .collect();
                assert_eq!(
                    cell(Some(got.join(";")).filter(|g| !g.is_empty())),
                    f[6],
                    "{}: files",
                    f[0]
                );
                let unresolved: Vec<&str> = v["unresolved"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|u| u["path"].as_str().unwrap())
                    .collect();
                assert_eq!(
                    cell(Some(unresolved.join(",")).filter(|g| !g.is_empty())),
                    f[7],
                    "{}: unresolved",
                    f[0]
                );
                let total = &v["total"];
                let got_total = if total.is_null() {
                    "-".to_string()
                } else {
                    [
                        "linesToCover",
                        "uncoveredLines",
                        "conditionsToCover",
                        "uncoveredConditions",
                        "lineCoverage",
                        "branchCoverage",
                        "coverage",
                    ]
                    .iter()
                    .map(|k| num(&total[*k]))
                    .collect::<Vec<_>>()
                    .join(",")
                };
                assert_eq!(got_total, f[8], "{}: total", f[0]);
            }
            rows += 1;
        }
        assert_eq!(rows, 21);
    }
}
