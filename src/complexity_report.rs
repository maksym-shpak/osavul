//! Пофайловий JSON-звіт складності `.ts` (цикломатичної або когнітивної) для коміту чи диска.

use serde::Serialize;

use crate::git_tree::{self, ReadError};
use crate::local_tree::{self, InputKind};
use crate::ts_cognitive;
use crate::ts_cyclomatic;
use crate::ts_loc;

/// Шлях і результат читання його вмісту.
pub struct FileInput {
    pub path: Vec<u8>,
    pub content: Result<Vec<u8>, ReadError>,
}

pub struct Report {
    /// Один компактний JSON із завершальним LF.
    pub json: Vec<u8>,
    pub complete: bool,
}

/// Обрана методика: визначає лічильник, `methodology.id` і ключ бала.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    Cyclomatic,
    Cognitive,
    Loc,
}

impl Metric {
    fn id(self) -> &'static str {
        match self {
            Metric::Cyclomatic => "ts-cyclomatic",
            Metric::Cognitive => "ts-cognitive",
            Metric::Loc => "ts-loc",
        }
    }

    fn score(self, path: &[u8], content: &[u8]) -> Result<u64, crate::ts_source::ScoreError> {
        match self {
            Metric::Cyclomatic => ts_cyclomatic::score(path, content),
            Metric::Cognitive => ts_cognitive::score(path, content),
            Metric::Loc => ts_loc::score(path, content),
        }
    }

    fn value(self, score: Option<u64>) -> ScoreField {
        match self {
            Metric::Cyclomatic => ScoreField::Cyclomatic { cyclomatic: score },
            Metric::Cognitive => ScoreField::Cognitive { cognitive: score },
            Metric::Loc => ScoreField::Loc { loc: score },
        }
    }
}

/// Рівно один ключ бала на місці поля запису; `null` зберігається явно.
#[derive(Serialize)]
#[serde(untagged)]
enum ScoreField {
    Cyclomatic { cyclomatic: Option<u64> },
    Cognitive { cognitive: Option<u64> },
    Loc { loc: Option<u64> },
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
struct FileRecord {
    path: Option<String>,
    #[serde(rename = "pathBytes", skip_serializing_if = "Option::is_none")]
    path_bytes: Option<String>,
    #[serde(flatten)]
    score: ScoreField,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
}

#[derive(Serialize)]
struct Document<'a> {
    sha: Option<&'a str>,
    source: &'static str,
    methodology: Methodology,
    scope: Scope,
    complete: bool,
    files: Vec<FileRecord>,
    total: Option<u64>,
}

/// Походження даних звіту; воно однозначно задає `sha` і `scope.input`.
pub enum Origin<'a> {
    Commit(&'a str),
    File,
    Directory,
}

/// Будує звіт із готових даних. Порядок `files` збігається з порядком входу.
pub fn build(origin: Origin<'_>, metric: Metric, files: Vec<FileInput>) -> Report {
    let (sha, input) = match origin {
        Origin::Commit(sha) => (Some(sha), "git-tree"),
        Origin::File => (None, "file"),
        Origin::Directory => (None, "directory"),
    };
    let mut records = Vec::with_capacity(files.len());
    let mut total = Some(0u64);
    for file in files {
        let (path, path_bytes) = match std::str::from_utf8(&file.path) {
            Ok(text) => (Some(text.to_string()), None),
            Err(_) => (None, Some(hex(&file.path))),
        };
        let outcome = match &file.content {
            Ok(content) => metric.score(&file.path, content).map_err(|e| e.reason()),
            Err(e) => Err(e.reason()),
        };
        match outcome {
            Ok(score) => {
                total = total.map(|t| t + score);
                records.push(FileRecord {
                    path,
                    path_bytes,
                    score: metric.value(Some(score)),
                    error: None,
                });
            }
            Err(reason) => {
                total = None;
                records.push(FileRecord {
                    path,
                    path_bytes,
                    score: metric.value(None),
                    error: Some(reason),
                });
            }
        }
    }
    let complete = total.is_some();
    let doc = Document {
        sha,
        source: "osavul",
        methodology: Methodology {
            id: metric.id(),
            version: 1,
        },
        scope: Scope {
            input,
            entries: "regular-files",
            suffix: ".ts",
        },
        complete,
        files: records,
        total,
    };
    let mut json =
        serde_json::to_vec(&doc).expect("серіалізація звіту не може завершитись помилкою");
    json.push(b'\n');
    Report { json, complete }
}

/// Вимірює коміт `rev` у поточному Git-репозиторії. `Err` — фатальна відмова до отримання звіту.
pub fn measure(rev: &str, metric: Metric) -> Result<Report, String> {
    let inventory = git_tree::inventory(rev)?;
    let contents = git_tree::read_contents(&inventory.entries);
    let files = inventory
        .entries
        .into_iter()
        .zip(contents)
        .map(|(entry, content)| FileInput {
            path: entry.path,
            content,
        })
        .collect();
    Ok(build(Origin::Commit(&inventory.sha), metric, files))
}

/// Вимірює поточний вміст локального файла або теки. `Err` — фатальна відмова обходу.
pub fn measure_local(path: &std::path::Path, metric: Metric) -> Result<Report, String> {
    let (kind, files) = local_tree::collect(path)?;
    let origin = match kind {
        InputKind::File => Origin::File,
        InputKind::Directory => Origin::Directory,
    };
    Ok(build(origin, metric, files))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CY: Metric = Metric::Cyclomatic;
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn file(path: &[u8], content: &[u8]) -> FileInput {
        FileInput {
            path: path.to_vec(),
            content: Ok(content.to_vec()),
        }
    }

    fn json(report: &Report) -> serde_json::Value {
        serde_json::from_slice(&report.json).unwrap()
    }

    #[test]
    fn empty_inventory_is_complete_with_zero_total() {
        let report = build(Origin::Commit(SHA), CY, vec![]);
        assert!(report.complete);
        assert_eq!(
            String::from_utf8(report.json).unwrap(),
            format!(
                "{{\"sha\":\"{SHA}\",\"source\":\"osavul\",\"methodology\":{{\"id\":\"ts-cyclomatic\",\"version\":1}},\"scope\":{{\"input\":\"git-tree\",\"entries\":\"regular-files\",\"suffix\":\".ts\"}},\"complete\":true,\"files\":[],\"total\":0}}\n"
            )
        );
    }

    #[test]
    fn complete_report_keeps_zeros_and_sums_all() {
        let report = build(
            Origin::Commit(SHA),
            CY,
            vec![
                file(b"a.ts", b"const x = 1;"),
                file(b"b.ts", b"function f() { if (a) {} }"),
            ],
        );
        let v = json(&report);
        assert!(report.complete);
        assert_eq!(v["files"][0]["cyclomatic"], 0);
        assert_eq!(v["files"][1]["cyclomatic"], 2);
        assert_eq!(v["total"], 2);
    }

    #[test]
    fn read_error_keeps_path_and_makes_report_incomplete() {
        let report = build(
            Origin::Commit(SHA),
            CY,
            vec![
                file(b"a.ts", b"function f() {}"),
                FileInput {
                    path: b"b.ts".to_vec(),
                    content: Err(ReadError),
                },
            ],
        );
        let v = json(&report);
        assert!(!report.complete);
        assert_eq!(v["complete"], false);
        assert_eq!(v["total"], serde_json::Value::Null);
        assert_eq!(v["files"][0]["cyclomatic"], 1);
        assert_eq!(v["files"][1]["path"], "b.ts");
        assert_eq!(v["files"][1]["cyclomatic"], serde_json::Value::Null);
        assert_eq!(v["files"][1]["error"], "read-error");
    }

    #[test]
    fn utf8_path_is_escaped_and_roundtrips() {
        let path = "q\"\\\n\u{1}є.ts";
        let report = build(Origin::Commit(SHA), CY, vec![file(path.as_bytes(), b"")]);
        let text = String::from_utf8(report.json.clone()).unwrap();
        assert!(!text[..text.len() - 1].contains('\n'));
        assert_eq!(json(&report)["files"][0]["path"], path);
        assert!(json(&report)["files"][0].get("pathBytes").is_none());
    }

    #[test]
    fn non_utf8_path_uses_path_bytes_after_null_path() {
        let report = build(
            Origin::Commit(SHA),
            CY,
            vec![file(b"\xff/\xfe.ts", b"function f() {}")],
        );
        let text = String::from_utf8(report.json.clone()).unwrap();
        assert!(text.contains(
            "\"files\":[{\"path\":null,\"pathBytes\":\"ff2ffe2e7473\",\"cyclomatic\":1}]"
        ));
        assert!(report.complete);
    }

    #[test]
    fn local_origins_have_explicit_null_sha_and_input_kind() {
        for (origin, input) in [(Origin::File, "file"), (Origin::Directory, "directory")] {
            let report = build(origin, CY, vec![]);
            let v = json(&report);
            assert!(v.as_object().unwrap().contains_key("sha"));
            assert!(v["sha"].is_null());
            assert_eq!(v["scope"]["input"], input);
            let text = String::from_utf8(report.json).unwrap();
            assert!(text.starts_with("{\"sha\":null,\"source\":\"osavul\""));
        }
    }

    #[test]
    fn cognitive_error_keeps_explicit_null_between_path_bytes_and_error() {
        let report = build(
            Origin::Commit(SHA),
            Metric::Cognitive,
            vec![FileInput {
                path: b"\xff.ts".to_vec(),
                content: Err(ReadError),
            }],
        );
        let text = String::from_utf8(report.json).unwrap();
        assert!(text.contains("\"methodology\":{\"id\":\"ts-cognitive\",\"version\":1}"));
        assert!(text.contains(
            "\"files\":[{\"path\":null,\"pathBytes\":\"ff2e7473\",\"cognitive\":null,\"error\":\"read-error\"}]"
        ));
        assert!(!text.contains("cyclomatic\":"));
    }

    #[test]
    fn cognitive_scores_use_cognitive_key_and_sum() {
        let report = build(
            Origin::Commit(SHA),
            Metric::Cognitive,
            vec![file(b"a.ts", b"function f() { if (a) {} }")],
        );
        let v = json(&report);
        assert_eq!(v["files"][0]["cognitive"], 1);
        assert!(v["files"][0].get("cyclomatic").is_none());
        assert_eq!(v["total"], 1);
    }

    #[test]
    fn loc_error_keeps_explicit_null_between_path_bytes_and_error() {
        let report = build(
            Origin::Commit(SHA),
            Metric::Loc,
            vec![FileInput {
                path: b"\xff.ts".to_vec(),
                content: Err(ReadError),
            }],
        );
        let text = String::from_utf8(report.json).unwrap();
        assert!(text.contains("\"methodology\":{\"id\":\"ts-loc\",\"version\":1}"));
        assert!(text.contains(
            "\"files\":[{\"path\":null,\"pathBytes\":\"ff2e7473\",\"loc\":null,\"error\":\"read-error\"}]"
        ));
        assert!(text.contains("\"complete\":false"));
        assert!(text.contains("\"total\":null"));
    }

    #[test]
    fn loc_scores_use_loc_key_and_sum_with_zero() {
        let report = build(
            Origin::Commit(SHA),
            Metric::Loc,
            vec![file(b"a.ts", b"// c\n"), file(b"b.ts", b"let a;\nlet b;\n")],
        );
        let v = json(&report);
        assert_eq!(v["files"][0]["loc"], 0);
        assert_eq!(v["files"][1]["loc"], 2);
        assert!(v["files"][1].get("cyclomatic").is_none());
        assert_eq!(v["complete"], true);
        assert_eq!(v["total"], 2);
    }
}
