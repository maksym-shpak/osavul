//! Зіставлення шляхів `SF` зі звіту LCOV з повним інвентарем `.ts` і явними тестовими виключеннями.
//!
//! Усе зіставлення лексичне: файлів на диску ніхто не відкриває, тож ключі старого коміту працюють без checkout.

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsStr;

use serde::Serialize;

use crate::complexity_report::{FileInput, Origin, Report, path_fields};
use crate::duplication_report::density_tenths;
use crate::lcov::{self, Counts, Defect};
use crate::ts_coverage;

type Comps = Vec<Vec<u8>>;

/// Хто володіє блоком `SF`.
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
    root: Comps,
    index: HashMap<Vec<u8>, usize>,
    tests: Vec<Vec<u8>>,
}

fn split(path: &[u8]) -> impl Iterator<Item = &[u8]> {
    path.split(|b| *b == b'/').filter(|c| !c.is_empty())
}

/// Лексично дописує `rel` до `base`, враховуючи `.` і `..`. Без `keep_up` вихід над корінь файлової системи
/// ігнорується, з ним — лишається ведучими `..`.
fn normalize(base: &[Vec<u8>], rel: &[u8], keep_up: bool) -> Comps {
    let mut out: Comps = base.to_vec();
    for part in split(rel) {
        match part {
            b"." => {}
            b".." => match out.last() {
                Some(last) if last.as_slice() != b".." => {
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
    args.iter()
        .map(|arg| {
            let bytes = os_bytes(arg);
            if bytes.first() == Some(&b'/') {
                return Err(format!(
                    "--tests: шлях має бути відносним: {}",
                    arg.to_string_lossy()
                ));
            }
            let comps = normalize(&[], &bytes, true);
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
    /// `root` — абсолютна канонічна тека, `keys` — повний інвентар у його порядку, `tests` — результат `parse_tests`.
    pub fn new<'a>(
        root: &[u8],
        keys: impl IntoIterator<Item = &'a [u8]>,
        tests: Vec<Vec<u8>>,
    ) -> Self {
        let index = keys
            .into_iter()
            .enumerate()
            .map(|(i, k)| (k.to_vec(), i))
            .collect();
        Matcher {
            root: normalize(&[], root, false),
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
        let under_root = |full: Comps| {
            full.starts_with(&self.root)
                .then(|| full[self.root.len()..].join(&b'/'))
        };
        if sf.first() == Some(&b'/') {
            return under_root(normalize(&[], sf, false)).into_iter().collect();
        }
        let dir = normalize(&[], report_dir, false);
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
            .filter_map(|base| under_root(normalize(base, sf, false)))
            .collect()
    }

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
        let ts = normalize(&[], sf, true)
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
    let matcher = Matcher::new(root, files.iter().map(|f| f.path.as_slice()), tests);
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

/// Читає запитані LCOV з диска; шлях відраховується від поточної теки процесу, тека звіту канонізується.
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

/// Звіт для коміту `rev`; `tests` — результат `parse_tests`. `Err` — фатальна відмова до публікації.
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

    fn matcher(inventory: &str, tests: &str) -> Matcher {
        let tests: Vec<&OsStr> = tests
            .split(',')
            .filter(|t| *t != "-")
            .map(OsStr::new)
            .collect();
        Matcher::new(
            ROOT.as_bytes(),
            inventory.split(',').map(str::as_bytes),
            parse_tests(&tests).unwrap(),
        )
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
        let ok = parse_tests(&[OsStr::new("test/"), OsStr::new("./a/../b.ts")]).unwrap();
        assert_eq!(ok, vec![b"test".to_vec(), b"b.ts".to_vec()]);
        assert!(parse_tests(&[OsStr::new("/abs")]).is_err());
        assert!(parse_tests(&[OsStr::new("..")]).is_err());
        assert!(parse_tests(&[OsStr::new("a/../../b")]).is_err());
    }

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
            let text = fragment(&dir, file, name).replace("{root}", ROOT);
            let report = lcov::parse(text.as_bytes());
            assert_eq!(report.blocks.len(), 1, "{}", f[0]);
            let report_dir = match f[3] {
                "-" => format!("{ROOT}/{}", Path::new(file).parent().unwrap().display()),
                "!outside" => "/elsewhere/reports".to_string(),
                place => format!("{ROOT}/{place}"),
            };
            let inventory = if f[6] == "-" { DEFAULT_INVENTORY } else { f[6] };
            let m = matcher(inventory, f[4]);
            let keys: Vec<&str> = inventory.split(',').collect();
            let got = match m.resolve(report_dir.as_bytes(), &report.blocks[0].path) {
                Owner::Main(i) => format!("main:{}", keys[i]),
                Owner::Ignored => "ignored".to_string(),
                Owner::Unresolved => "unresolved".to_string(),
            };
            assert_eq!(got, f[5], "{}", f[0]);
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
            let reports: Vec<LcovInput> = inputs
                .split(',')
                .filter(|i| *i != "-")
                .map(|input| {
                    let (file, name) = input.split_once('#').unwrap();
                    let text = fragment(&dir, file, name).replace("{root}", ROOT);
                    let place = Path::new(file).parent().unwrap().display();
                    lcov_in(&format!("{ROOT}/{place}"), &text)
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
            let tests: Vec<&OsStr> = f[4]
                .split(',')
                .filter(|t| *t != "-")
                .map(OsStr::new)
                .collect();
            let report = build(
                Origin::Directory,
                ROOT.as_bytes(),
                files,
                &reports,
                parse_tests(&tests).unwrap(),
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
            rows += 1;
        }
        assert_eq!(rows, 21);
    }
}
