//! Пофайловий JSON-звіт цикломатичної складності `.ts` для коміту.

use serde::Serialize;

use crate::git_tree::{self, ReadError};
use crate::ts_cyclomatic;

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
    cyclomatic: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
}

#[derive(Serialize)]
struct Document<'a> {
    sha: &'a str,
    source: &'static str,
    methodology: Methodology,
    scope: Scope,
    complete: bool,
    files: Vec<FileRecord>,
    total: Option<u64>,
}

/// Будує звіт із готових даних. Порядок `files` збігається з порядком входу.
pub fn build(sha: &str, files: Vec<FileInput>) -> Report {
    let mut records = Vec::with_capacity(files.len());
    let mut total = Some(0u64);
    for file in files {
        let (path, path_bytes) = match std::str::from_utf8(&file.path) {
            Ok(text) => (Some(text.to_string()), None),
            Err(_) => (None, Some(hex(&file.path))),
        };
        let outcome = match &file.content {
            Ok(content) => ts_cyclomatic::score(&file.path, content).map_err(|e| e.reason()),
            Err(e) => Err(e.reason()),
        };
        match outcome {
            Ok(score) => {
                total = total.map(|t| t + score);
                records.push(FileRecord { path, path_bytes, cyclomatic: Some(score), error: None });
            }
            Err(reason) => {
                total = None;
                records.push(FileRecord { path, path_bytes, cyclomatic: None, error: Some(reason) });
            }
        }
    }
    let complete = total.is_some();
    let doc = Document {
        sha,
        source: "osavul",
        methodology: Methodology { id: "ts-cyclomatic", version: 1 },
        scope: Scope { input: "git-tree", entries: "regular-files", suffix: ".ts" },
        complete,
        files: records,
        total,
    };
    let mut json = serde_json::to_vec(&doc).expect("серіалізація звіту не може завершитись помилкою");
    json.push(b'\n');
    Report { json, complete }
}

/// Вимірює коміт `rev` у поточному Git-репозиторії. `Err` — фатальна відмова до отримання звіту.
pub fn measure(rev: &str) -> Result<Report, String> {
    let inventory = git_tree::inventory(rev)?;
    let contents = git_tree::read_contents(&inventory.entries);
    let files = inventory
        .entries
        .into_iter()
        .zip(contents)
        .map(|(entry, content)| FileInput { path: entry.path, content })
        .collect();
    Ok(build(&inventory.sha, files))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn file(path: &[u8], content: &[u8]) -> FileInput {
        FileInput { path: path.to_vec(), content: Ok(content.to_vec()) }
    }

    fn json(report: &Report) -> serde_json::Value {
        serde_json::from_slice(&report.json).unwrap()
    }

    #[test]
    fn empty_inventory_is_complete_with_zero_total() {
        let report = build(SHA, vec![]);
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
            SHA,
            vec![file(b"a.ts", b"const x = 1;"), file(b"b.ts", b"function f() { if (a) {} }")],
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
            SHA,
            vec![
                file(b"a.ts", b"function f() {}"),
                FileInput { path: b"b.ts".to_vec(), content: Err(ReadError) },
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
        let report = build(SHA, vec![file(path.as_bytes(), b"")]);
        let text = String::from_utf8(report.json.clone()).unwrap();
        assert!(!text[..text.len() - 1].contains('\n'));
        assert_eq!(json(&report)["files"][0]["path"], path);
        assert!(json(&report)["files"][0].get("pathBytes").is_none());
    }

    #[test]
    fn non_utf8_path_uses_path_bytes_after_null_path() {
        let report = build(SHA, vec![file(b"\xff/\xfe.ts", b"function f() {}")]);
        let text = String::from_utf8(report.json.clone()).unwrap();
        assert!(text.contains("\"files\":[{\"path\":null,\"pathBytes\":\"ff2ffe2e7473\",\"cyclomatic\":1}]"));
        assert!(report.complete);
    }
}
