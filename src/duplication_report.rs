//! Звіт дублювання `.ts` (методика `ts-duplication`, версія 1) для коміту чи диска.

use std::path::Path;

use serde::Serialize;

use crate::complexity_report::{FileInput, Origin, Report, git_inputs, local_inputs, path_fields};
use crate::ts_duplication::{self, Group, Prepared};

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
    lines: Option<u64>,
    #[serde(rename = "duplicatedLines")]
    duplicated_lines: Option<u64>,
    #[serde(rename = "duplicatedBlocks")]
    duplicated_blocks: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
}

#[derive(Serialize)]
struct Block {
    path: Option<String>,
    #[serde(rename = "pathBytes", skip_serializing_if = "Option::is_none")]
    path_bytes: Option<String>,
    #[serde(rename = "startLine")]
    start_line: usize,
    #[serde(rename = "endLine")]
    end_line: usize,
}

#[derive(Serialize)]
struct CloneGroup {
    blocks: Vec<Block>,
}

/// Для порожнього інвентарю — ціле `0`, інакше число з одним десятковим знаком.
#[derive(Serialize)]
#[serde(untagged)]
enum Density {
    Zero(u64),
    Percent(f64),
}

#[derive(Serialize)]
struct Total {
    lines: u64,
    #[serde(rename = "duplicatedLines")]
    duplicated_lines: u64,
    #[serde(rename = "duplicatedBlocks")]
    duplicated_blocks: u64,
    #[serde(rename = "duplicatedFiles")]
    duplicated_files: u64,
    density: Density,
}

#[derive(Serialize)]
struct Document<'a> {
    sha: Option<&'a str>,
    source: &'static str,
    methodology: Methodology,
    scope: Scope,
    complete: bool,
    files: Vec<FileRecord>,
    duplications: Vec<CloneGroup>,
    total: Option<Total>,
}

/// Десяті частки відсотка з округленням точної половини вгору (ціла арифметика).
pub(crate) fn density_tenths(duplicated: u64, lines: u64) -> u64 {
    let (d, l) = (u128::from(duplicated), u128::from(lines));
    ((2000 * d + l) / (2 * l)) as u64
}

/// Розмір об'єднання включних діапазонів `(початок, кінець)`.
fn union_len(mut ranges: Vec<(usize, usize)>) -> u64 {
    ranges.sort_unstable();
    let mut total = 0u64;
    let mut covered_to: Option<usize> = None;
    for (start, end) in ranges {
        let from = covered_to.map_or(start, |c| start.max(c + 1));
        if end >= from {
            total += (end - from + 1) as u64;
            covered_to = Some(end);
        }
    }
    total
}

/// Внесок успішно підготовленого файла.
pub(crate) struct Contribution {
    pub lines: u64,
    pub duplicated_lines: u64,
    pub duplicated_blocks: u64,
}

/// Результат спільного пошуку дублів одного боку до серіалізації.
pub(crate) struct Analysis {
    /// За порядком входу: внесок або причина відмови файла.
    pub files: Vec<Result<Contribution, &'static str>>,
    /// Групи з однобазовими включними межами `(індекс файла, початок, кінець)` у канонічному
    /// порядку: входження за байтами шляху й межами, групи за послідовністю входжень.
    pub groups: Vec<Vec<(usize, usize, usize)>>,
    pub lines: u64,
    pub duplicated_lines: u64,
    pub duplicated_blocks: u64,
    pub duplicated_files: u64,
}

/// Готує файли, шукає групи спільно серед успішних і рахує внески. Відомі суми охоплюють лише
/// успішні файли, тож для неповного входу вони не дорівнюють публічному `total`.
pub(crate) fn analyze(files: &[FileInput]) -> Analysis {
    let prepared: Vec<Result<Prepared, &'static str>> = files
        .iter()
        .map(|file| match &file.content {
            Ok(content) => ts_duplication::prepare(&file.path, content).map_err(|e| e.reason()),
            Err(e) => Err(e.reason()),
        })
        .collect();

    // `ok[i]` — індекс у `files` для i-го успішного файла.
    let mut ok: Vec<usize> = Vec::new();
    let mut refs: Vec<&Prepared> = Vec::new();
    for (i, p) in prepared.iter().enumerate() {
        if let Ok(p) = p {
            ok.push(i);
            refs.push(p);
        }
    }
    let found: Vec<Group> = ts_duplication::find_groups(&refs);

    let mut groups: Vec<Vec<(usize, usize, usize)>> = found
        .iter()
        .map(|g| {
            let mut blocks: Vec<(usize, usize, usize)> = g
                .occurrences
                .iter()
                .map(|o| (ok[o.file], o.start_line + 1, o.end_line + 1))
                .collect();
            blocks.sort_by(|a, b| {
                files[a.0]
                    .path
                    .cmp(&files[b.0].path)
                    .then(a.1.cmp(&b.1))
                    .then(a.2.cmp(&b.2))
            });
            blocks
        })
        .collect();
    let key = |b: &(usize, usize, usize)| (files[b.0].path.clone(), b.1, b.2);
    groups.sort_by(|a, b| a.iter().map(key).cmp(b.iter().map(key)));

    let mut blocks_of: Vec<u64> = vec![0; files.len()];
    let mut ranges_of: Vec<Vec<(usize, usize)>> = vec![Vec::new(); files.len()];
    for &(file, start, end) in groups.iter().flatten() {
        blocks_of[file] += 1;
        ranges_of[file].push((start, end));
    }

    let mut analysis = Analysis {
        files: Vec::with_capacity(files.len()),
        groups,
        lines: 0,
        duplicated_lines: 0,
        duplicated_blocks: 0,
        duplicated_files: 0,
    };
    for (i, p) in prepared.iter().enumerate() {
        match p {
            Ok(p) => {
                let dup = union_len(std::mem::take(&mut ranges_of[i]));
                analysis.lines += p.lines;
                analysis.duplicated_lines += dup;
                analysis.duplicated_blocks += blocks_of[i];
                analysis.duplicated_files += u64::from(blocks_of[i] > 0);
                analysis.files.push(Ok(Contribution {
                    lines: p.lines,
                    duplicated_lines: dup,
                    duplicated_blocks: blocks_of[i],
                }));
            }
            Err(reason) => analysis.files.push(Err(reason)),
        }
    }
    analysis
}

/// Будує звіт із готових даних. Порядок `files` збігається з порядком входу.
///
/// Файл, який не вдалося прочитати чи розібрати, дістає `null` у числах і причину в `error`, а
/// `complete` і `total` стають `false` та `null`. Решта файлів однаково беруть участь у пошуку
/// дублів.
pub fn build(origin: Origin<'_>, files: Vec<FileInput>) -> Report {
    let (sha, input) = origin.fields();
    let analysis = analyze(&files);
    let complete = analysis.files.iter().all(Result::is_ok);
    let groups = &analysis.groups;

    let mut total = Total {
        lines: analysis.lines,
        duplicated_lines: analysis.duplicated_lines,
        duplicated_blocks: analysis.duplicated_blocks,
        duplicated_files: analysis.duplicated_files,
        density: Density::Zero(0),
    };
    let mut records = Vec::with_capacity(files.len());
    for (file, outcome) in files.iter().zip(&analysis.files) {
        let (path, path_bytes) = path_fields(&file.path);
        records.push(match outcome {
            Ok(c) => FileRecord {
                path,
                path_bytes,
                lines: Some(c.lines),
                duplicated_lines: Some(c.duplicated_lines),
                duplicated_blocks: Some(c.duplicated_blocks),
                error: None,
            },
            Err(reason) => FileRecord {
                path,
                path_bytes,
                lines: None,
                duplicated_lines: None,
                duplicated_blocks: None,
                error: Some(reason),
            },
        });
    }
    if complete && !files.is_empty() {
        let tenths = density_tenths(total.duplicated_lines, total.lines);
        total.density = Density::Percent(tenths as f64 / 10.0);
    }

    let duplications = groups
        .iter()
        .map(|g| CloneGroup {
            blocks: g
                .iter()
                .map(|&(file, start_line, end_line)| {
                    let (path, path_bytes) = path_fields(&files[file].path);
                    Block {
                        path,
                        path_bytes,
                        start_line,
                        end_line,
                    }
                })
                .collect(),
        })
        .collect();

    let doc = Document {
        sha,
        source: "osavul",
        methodology: Methodology {
            id: "ts-duplication",
            version: 1,
        },
        scope: Scope {
            input,
            entries: "regular-files",
            suffix: ".ts",
        },
        complete,
        files: records,
        duplications,
        total: complete.then_some(total),
    };
    let mut json =
        serde_json::to_vec(&doc).expect("серіалізація звіту не може завершитись помилкою");
    json.push(b'\n');
    Report { json, complete }
}

/// Звіт дублювання коміту `rev`. `Err` — фатальна відмова до отримання звіту.
pub fn measure(rev: &str) -> Result<Report, String> {
    let (sha, files) = git_inputs(rev)?;
    Ok(build(Origin::Commit(&sha), files))
}

/// Звіт дублювання поточного вмісту файла або теки на диску.
pub fn measure_local(path: &Path) -> Result<Report, String> {
    let (kind, files) = local_inputs(path)?;
    Ok(build(kind.into(), files))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_tree::ReadError;

    fn file(path: &[u8], content: &[u8]) -> FileInput {
        FileInput {
            path: path.to_vec(),
            content: Ok(content.to_vec()),
        }
    }

    fn json(report: &Report) -> serde_json::Value {
        serde_json::from_slice(&report.json).unwrap()
    }

    fn text(report: &Report) -> String {
        String::from_utf8(report.json.clone()).unwrap()
    }

    /// Десять рядків по десять токенів; `tag` відрізняє блоки між собою.
    fn block(tag: &str) -> String {
        (0..10)
            .map(|i| format!("{tag}{i} = b{i} + c{i} + d{i} + e{i};\n"))
            .collect()
    }

    #[test]
    fn empty_inventory_has_exact_bytes() {
        let report = build(Origin::Directory, vec![]);
        assert!(report.complete);
        assert_eq!(
            text(&report),
            "{\"sha\":null,\"source\":\"osavul\",\"methodology\":{\"id\":\"ts-duplication\",\"version\":1},\"scope\":{\"input\":\"directory\",\"entries\":\"regular-files\",\"suffix\":\".ts\"},\"complete\":true,\"files\":[],\"duplications\":[],\"total\":{\"lines\":0,\"duplicatedLines\":0,\"duplicatedBlocks\":0,\"duplicatedFiles\":0,\"density\":0}}\n"
        );
    }

    #[test]
    fn nonempty_without_duplicates_has_zero_point_zero() {
        let report = build(Origin::File, vec![file(b"a.ts", b"")]);
        assert_eq!(
            text(&report),
            "{\"sha\":null,\"source\":\"osavul\",\"methodology\":{\"id\":\"ts-duplication\",\"version\":1},\"scope\":{\"input\":\"file\",\"entries\":\"regular-files\",\"suffix\":\".ts\"},\"complete\":true,\"files\":[{\"path\":\"a.ts\",\"lines\":1,\"duplicatedLines\":0,\"duplicatedBlocks\":0}],\"duplications\":[],\"total\":{\"lines\":1,\"duplicatedLines\":0,\"duplicatedBlocks\":0,\"duplicatedFiles\":0,\"density\":0.0}}\n"
        );
        let report = build(Origin::File, vec![file(b"a.ts", b"\n")]);
        assert_eq!(json(&report)["files"][0]["lines"], 2);
    }

    // Виявляє зсув округлення густини: точна половина десятої має йти вгору, а не відкидатися.
    #[test]
    fn density_rounds_half_up_to_tenths() {
        assert_eq!(density_tenths(1, 16), 63);
        assert_eq!(density_tenths(0, 5), 0);
        assert_eq!(density_tenths(5, 5), 1000);
        assert_eq!(density_tenths(1, 3), 333);
        assert_eq!(density_tenths(1, 8), 125);
        // Точна половина десятої округлюється вгору: 1 із 2000 рядків — це 0.05 %, тобто 0.1.
        assert_eq!(density_tenths(1, 2000), 1);
    }

    #[test]
    fn density_formats_as_one_decimal_number() {
        let b = block("a");
        let src = format!("{b}{b}");
        let report = build(Origin::File, vec![file(b"a.ts", src.as_bytes())]);
        // Блоки 1–10 та 11–20 суміжні, поза ними один порожній рядок.
        assert!(text(&report).ends_with("\"density\":95.2}}\n"));
        let src = format!("{b}x;\n{b}");
        let report = build(Origin::File, vec![file(b"a.ts", src.as_bytes())]);
        assert!(text(&report).contains("\"lines\":22,"));
    }

    #[test]
    fn union_counts_overlaps_once() {
        assert_eq!(union_len(vec![(1, 10), (5, 14)]), 14);
        assert_eq!(union_len(vec![(1, 3), (4, 6)]), 6);
        assert_eq!(union_len(vec![(1, 10), (3, 4)]), 10);
        assert_eq!(union_len(vec![]), 0);
    }

    #[test]
    fn blocks_cover_comments_and_blank_lines_inside() {
        let a = "// x\n";
        let mut body = block("a");
        body = body.replacen('\n', "\n\n// inside\n", 1);
        let src = format!("{a}{body}k1 = 1;\n{}", block("a"));
        let report = build(Origin::File, vec![file(b"a.ts", src.as_bytes())]);
        let v = json(&report);
        let blocks = &v["duplications"][0]["blocks"];
        assert_eq!(blocks[0]["startLine"], 2);
        assert_eq!(blocks[0]["endLine"], 13);
        assert_eq!(v["files"][0]["duplicatedBlocks"], 2);
        assert_eq!(v["files"][0]["duplicatedLines"], 12 + 10);
        assert_eq!(v["total"]["duplicatedFiles"], 1);
    }

    // Виявляє порушення порядку файлів, блоків і груп (шлях за байтами) та недетермінізм: два
    // побудови з однаковим входом мають дати однакові байти.
    #[test]
    fn files_blocks_and_groups_are_sorted_with_path_bytes() {
        let b = block("a");
        let other = block("z");
        let report = build(
            Origin::Commit("0123456789abcdef0123456789abcdef01234567"),
            vec![
                file(b"a.ts", b.as_bytes()),
                file(b"b.ts", other.as_bytes()),
                file(b"c.ts", other.as_bytes()),
                file(b"\xff.ts", b.as_bytes()),
            ],
        );
        let t = text(&report);
        assert!(t.starts_with("{\"sha\":\"0123456789abcdef0123456789abcdef01234567\",\"source\""));
        assert!(t.contains("\"files\":[{\"path\":\"a.ts\""));
        assert!(t.contains("{\"path\":null,\"pathBytes\":\"ff2e7473\",\"lines\":11"));
        assert!(t.contains(
            "\"duplications\":[{\"blocks\":[{\"path\":\"a.ts\",\"startLine\":1,\"endLine\":10},{\"path\":null,\"pathBytes\":\"ff2e7473\",\"startLine\":1,\"endLine\":10}]},{\"blocks\":[{\"path\":\"b.ts\""
        ));
        assert!(t.ends_with("}\n"));
        assert_eq!(t.matches('\n').count(), 1);
        let again = build(
            Origin::Commit("0123456789abcdef0123456789abcdef01234567"),
            vec![
                file(b"a.ts", b.as_bytes()),
                file(b"b.ts", other.as_bytes()),
                file(b"c.ts", other.as_bytes()),
                file(b"\xff.ts", b.as_bytes()),
            ],
        );
        assert_eq!(report.json, again.json);
    }

    // Виявляє втрату дублів зі справних файлів поруч із помилковими, хибний повний звіт та підміну
    // `null` у числах помилкового файла.
    #[test]
    fn errors_keep_nulls_next_to_valid_groups() {
        let b = block("a");
        let report = build(
            Origin::File,
            vec![
                file(b"a.ts", b.as_bytes()),
                file(b"b.ts", b.as_bytes()),
                file(b"bad.ts", b"let a = ;"),
                file(b"bin.ts", b"\xff"),
                FileInput {
                    path: b"gone.ts".to_vec(),
                    content: Err(ReadError),
                },
            ],
        );
        let v = json(&report);
        assert!(!report.complete);
        assert_eq!(v["complete"], false);
        assert!(v["total"].is_null());
        let analysis = analyze(&[
            file(b"a.ts", b.as_bytes()),
            file(b"b.ts", b.as_bytes()),
            file(b"bad.ts", b"let a = ;"),
            file(b"bin.ts", b"\xff"),
            FileInput {
                path: b"gone.ts".to_vec(),
                content: Err(ReadError),
            },
        ]);
        assert_eq!(analysis.duplicated_lines, 20);
        assert_eq!(analysis.groups.len(), 1);
        assert_eq!(analysis.groups[0].len(), 2);
        for i in [0, 1] {
            assert_eq!(analysis.files[i].as_ref().unwrap().duplicated_lines, 10);
        }
        let reasons: Vec<_> = analysis.files[2..]
            .iter()
            .map(|f| f.as_ref().err().copied())
            .collect();
        assert_eq!(
            reasons,
            [Some("syntax-error"), Some("invalid-utf8"), Some("read-error")]
        );
        assert_eq!(v["duplications"].as_array().unwrap().len(), 1);
        assert_eq!(v["files"][0]["duplicatedLines"], 10);
        let t = text(&report);
        for (p, e) in [
            ("bad.ts", "syntax-error"),
            ("bin.ts", "invalid-utf8"),
            ("gone.ts", "read-error"),
        ] {
            assert!(t.contains(&format!(
                "{{\"path\":\"{p}\",\"lines\":null,\"duplicatedLines\":null,\"duplicatedBlocks\":null,\"error\":\"{e}\"}}"
            )));
        }
        assert!(t.contains("\"total\":null"));
    }

    #[test]
    fn all_files_failing_gives_incomplete_empty_report() {
        let report = build(Origin::File, vec![file(b"a.ts", b"\xff")]);
        let v = json(&report);
        assert_eq!(v["complete"], false);
        assert_eq!(v["duplications"].as_array().unwrap().len(), 0);
        assert!(v["total"].is_null());
    }

    #[test]
    fn corpus_matches_expected_tsv() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts-duplication");
        let table = std::fs::read_to_string(dir.join("expected.tsv")).unwrap();
        let mut rows = 0;
        for line in table.lines().skip(1) {
            let cols: Vec<&str> = line.split('\t').collect();
            let name = cols[0];
            let inputs: Vec<FileInput> = cols[2]
                .split(',')
                .map(|p| file(p.as_bytes(), &std::fs::read(dir.join(p)).unwrap()))
                .collect();
            let report = build(Origin::Directory, inputs);
            let v = json(&report);
            assert_eq!(v["complete"], true, "{name}");

            let groups: Vec<String> = v["duplications"]
                .as_array()
                .unwrap()
                .iter()
                .map(|g| {
                    g["blocks"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|b| {
                            format!(
                                "{}:{}-{}",
                                b["path"].as_str().unwrap(),
                                b["startLine"],
                                b["endLine"]
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect();
            let groups = if groups.is_empty() {
                "-".to_string()
            } else {
                groups.join(";")
            };
            assert_eq!(groups, cols[3], "{name}: groups");

            let files: Vec<String> = v["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    format!(
                        "{}:{}:{}:{}",
                        f["path"].as_str().unwrap(),
                        f["lines"],
                        f["duplicatedLines"],
                        f["duplicatedBlocks"]
                    )
                })
                .collect();
            assert_eq!(files.join(" "), cols[4], "{name}: files");

            let t = &v["total"];
            let total = format!(
                "{}:{}:{}:{}:{}",
                t["lines"],
                t["duplicatedLines"],
                t["duplicatedBlocks"],
                t["duplicatedFiles"],
                t["density"]
            );
            assert_eq!(total, cols[5], "{name}: total");
            rows += 1;
        }
        assert_eq!(rows, 16);
    }
}
