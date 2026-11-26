//! Розбір звітів LCOV і злиття блоків `SF` одного файла в чотири лічильники покриття.
//!
//! Враховуються лише `SF`, `end_of_record`, `DA` і `BRDA`; усі інші записи (зокрема готові підсумки
//! `LF`/`LH`/`BRF`/`BRH`) ігноруються, а лічильники рахуються з `DA` і `BRDA`.

use std::cmp::Ordering;
use std::collections::BTreeMap;

type BranchKeys<'a> = BTreeMap<(usize, &'a [u8]), u64>;

/// Блок одного `SF`. Дефект запису лишається при блоці, щоб його власник визначався пізніше.
#[derive(Debug, Default)]
pub struct Block {
    pub path: Vec<u8>,
    invalid: bool,
    da: Vec<(i64, u64)>,
    brda: Vec<Branch>,
}

#[derive(Debug)]
struct Branch {
    line: i64,
    block: Vec<u8>,
    branch: Vec<u8>,
    count: u64,
}

/// Результат розбору одного звіту.
#[derive(Debug, Default)]
pub struct Report {
    pub blocks: Vec<Block>,
    /// Є `DA` або `BRDA` поза блоком `SF`: такий запис не має власника.
    pub orphan: bool,
}

/// Дефект даних блоків; перевіряється в `check`.
#[derive(Debug, PartialEq, Eq)]
pub enum Defect {
    /// Запис `DA` чи `BRDA` не розібрався за формою полів.
    InvalidRecord,
    /// Номер рядка `DA` чи `BRDA` лежить поза `1..=max_line`.
    LineOutOfRange,
}

/// Чотири лічильники покриття одного файла після злиття блоків.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Default)]
pub struct Counts {
    /// Різні рядки, згадані в `DA` чи `BRDA`.
    pub lines: u64,
    /// Рядки із сумарним лічильником нуль (покрита умова рядка рахується як попадання).
    pub uncovered_lines: u64,
    /// Умови (гілки) після злиття; ключ умови — ранг блока й ідентифікатор гілки в межах рядка.
    pub conditions: u64,
    pub uncovered_conditions: u64,
}

/// Розбирає один звіт; не повертає помилок.
///
/// Дефект запису лишається в блоці й виявляється в `check`. `DA` чи `BRDA` поза блоком `SF`
/// лише ставлять `Report::orphan`. Блок закривається `end_of_record`, наступним `SF` або
/// кінцем звіту; `\r` перед `\n` відкидається.
pub fn parse(bytes: &[u8]) -> Report {
    let mut report = Report::default();
    let mut open: Option<Block> = None;
    for raw in bytes.split(|b| *b == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if let Some(path) = line.strip_prefix(b"SF:") {
            report.blocks.extend(open.take());
            open = Some(Block {
                path: path.to_vec(),
                ..Block::default()
            });
        } else if line == b"end_of_record" {
            report.blocks.extend(open.take());
        } else if let Some(rest) = tagged(line, b"DA") {
            match open.as_mut() {
                None => report.orphan = true,
                Some(block) => match parse_da(rest) {
                    Some(record) => block.da.push(record),
                    None => block.invalid = true,
                },
            }
        } else if let Some(rest) = tagged(line, b"BRDA") {
            match open.as_mut() {
                None => report.orphan = true,
                Some(block) => match parse_brda(rest) {
                    Some(record) => block.brda.push(record),
                    None => block.invalid = true,
                },
            }
        }
    }
    report.blocks.extend(open);
    report
}

/// Операнди запису `tag:`; рядок із самим `tag` без двокрапки — запис без полів (порожній
/// операнд).
fn tagged<'a>(line: &'a [u8], tag: &[u8]) -> Option<&'a [u8]> {
    if line == tag {
        return Some(b"");
    }
    line.strip_prefix(tag)?.strip_prefix(b":")
}

fn digits(field: &[u8]) -> bool {
    !field.is_empty() && field.iter().all(u8::is_ascii_digit)
}

/// Невід'ємне десяткове ціле; надвеликі значення насичуються, тож нуль і додатне не змішуються.
fn parse_count(field: &[u8]) -> Option<u64> {
    if !digits(field) {
        return None;
    }
    Some(field.iter().fold(0u64, |acc, d| {
        acc.saturating_mul(10).saturating_add(u64::from(d - b'0'))
    }))
}

/// Ціле зі знаком. Нерозбірне поле — `None`; число поза межами файла лишається цілим для
/// перевірки меж.
fn parse_line(field: &[u8]) -> Option<i64> {
    let (negative, body) = match field.strip_prefix(b"-") {
        Some(body) => (true, body),
        None => (false, field),
    };
    let magnitude = i64::try_from(parse_count(body)?).unwrap_or(i64::MAX);
    Some(if negative { -magnitude } else { magnitude })
}

fn parse_da(rest: &[u8]) -> Option<(i64, u64)> {
    let fields: Vec<&[u8]> = rest.split(|b| *b == b',').collect();
    // Третє поле — контрольна сума, вона ігнорується.
    if !(2..=3).contains(&fields.len()) {
        return None;
    }
    Some((parse_line(fields[0])?, parse_count(fields[1])?))
}

fn parse_brda(rest: &[u8]) -> Option<Branch> {
    let fields: Vec<&[u8]> = rest.split(|b| *b == b',').collect();
    if fields.len() != 4 || fields[1].is_empty() || fields[2].is_empty() {
        return None;
    }
    // `-` у лічильнику гілки означає, що гілку не виконано, тобто нуль.
    let count = if fields[3] == b"-" {
        0
    } else {
        parse_count(fields[3])?
    };
    Some(Branch {
        line: parse_line(fields[0])?,
        block: fields[1].to_vec(),
        branch: fields[2].to_vec(),
        count,
    })
}

/// Порядок ідентифікаторів блоків `BRDA`: спершу ті, що з самих цифр, за числовим значенням
/// (рівні значення, як-от `10` і `010`, — за байтами), потім решта за байтами.
fn cmp_block_id(a: &[u8], b: &[u8]) -> Ordering {
    fn significant(id: &[u8]) -> &[u8] {
        let skip = id.iter().take_while(|d| **d == b'0').count();
        &id[skip..]
    }
    match (digits(a), digits(b)) {
        (true, true) => {
            let (x, y) = (significant(a), significant(b));
            x.len()
                .cmp(&y.len())
                .then_with(|| x.cmp(y))
                .then_with(|| a.cmp(b))
        }
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a.cmp(b),
    }
}

/// Дефект даних блоків: нерозбірний запис має пріоритет над номером рядка поза `1..=max_line`.
pub fn check(blocks: &[&Block], max_line: usize) -> Result<(), Defect> {
    if blocks.iter().any(|b| b.invalid) {
        return Err(Defect::InvalidRecord);
    }
    let max = i64::try_from(max_line).unwrap_or(i64::MAX);
    let in_range = |line: i64| (1..=max).contains(&line);
    let bad = blocks.iter().any(|b| {
        b.da.iter().any(|(line, _)| !in_range(*line)) || b.brda.iter().any(|r| !in_range(r.line))
    });
    if bad {
        return Err(Defect::LineOutOfRange);
    }
    Ok(())
}

/// Зливає всі блоки одного файла в `Counts`. `max_line` — число фізичних рядків файла.
///
/// Спершу виконується `check`, тож дефект даних не дає часткового результату.
/// Лічильники `DA` одного рядка підсумовуються між блоками. Умова рахується непокритою, лише
/// якщо жоден блок її не попав і вона оголошена в кожному блоці, що має `BRDA` цього рядка.
pub fn merge(blocks: &[&Block], max_line: usize) -> Result<Counts, Defect> {
    check(blocks, max_line)?;
    let mut line_counts: BTreeMap<i64, u64> = BTreeMap::new();
    // Для кожного рядка — ключі гілок кожного блока `SF`, який має `BRDA` цього рядка.
    let mut per_line: BTreeMap<i64, Vec<BranchKeys>> = BTreeMap::new();
    for block in blocks {
        for &(line, count) in &block.da {
            let total = line_counts.entry(line).or_insert(0);
            *total = total.saturating_add(count);
        }
        let mut by_line: BTreeMap<i64, Vec<&Branch>> = BTreeMap::new();
        for record in &block.brda {
            line_counts.entry(record.line).or_insert(0);
            by_line.entry(record.line).or_default().push(record);
        }
        for (line, records) in by_line {
            // Ідентифікатори блоків зіставляються між `SF` за рангом у межах рядка, а не за
            // текстом (див. `ranks_are_local_to_line_and_block`).
            let mut ids: Vec<&[u8]> = records.iter().map(|r| r.block.as_slice()).collect();
            ids.sort_by(|a, b| cmp_block_id(a, b));
            ids.dedup();
            let mut keys = BranchKeys::new();
            for record in records {
                let rank = ids
                    .binary_search_by(|id| cmp_block_id(id, &record.block))
                    .unwrap_or_default();
                let total = keys.entry((rank, record.branch.as_slice())).or_insert(0);
                *total = total.saturating_add(record.count);
            }
            per_line.entry(line).or_default().push(keys);
        }
    }
    let mut counts = Counts::default();
    for (line, declared_in) in per_line {
        let blocks_with_branches = declared_in.len();
        let mut keys: BTreeMap<(usize, &[u8]), (bool, usize)> = BTreeMap::new();
        for block_keys in &declared_in {
            for (key, count) in block_keys {
                let entry = keys.entry(*key).or_insert((false, 0));
                entry.0 |= *count > 0;
                entry.1 += 1;
            }
        }
        let mut covered = 0u64;
        for (hit, declared) in keys.values() {
            if *hit {
                covered += 1;
            } else if *declared == blocks_with_branches {
                counts.uncovered_conditions += 1;
                counts.conditions += 1;
            }
        }
        counts.conditions += covered;
        // Покрита умова робить рядок покритим, навіть якщо `DA` дає нуль.
        if let Some(total) = line_counts.get_mut(&line) {
            *total = total.saturating_add(covered);
        }
    }
    for total in line_counts.values() {
        counts.lines += 1;
        if *total == 0 {
            counts.uncovered_lines += 1;
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn merged(reports: &[&str], limit: usize) -> Result<Counts, Defect> {
        let parsed: Vec<Report> = reports.iter().map(|r| parse(r.as_bytes())).collect();
        let blocks: Vec<&Block> = parsed.iter().flat_map(|r| r.blocks.iter()).collect();
        merge(&blocks, limit)
    }

    fn counts(l: u64, ul: u64, c: u64, uc: u64) -> Counts {
        Counts {
            lines: l,
            uncovered_lines: ul,
            conditions: c,
            uncovered_conditions: uc,
        }
    }

    #[test]
    fn lf_and_crlf_give_same_blocks() {
        let lf = "SF:a.ts\nDA:1,2\nBRDA:1,0,0,-\nend_of_record\n";
        let crlf = lf.replace('\n', "\r\n");
        assert_eq!(merged(&[lf], 5), merged(&[&crlf], 5));
        assert_eq!(merged(&[lf], 5), Ok(counts(1, 0, 1, 1)));
        assert_eq!(parse(crlf.as_bytes()).blocks[0].path, b"a.ts");
    }

    #[test]
    fn blocks_close_at_end_of_record_next_sf_and_report_end() {
        let r = parse(b"SF:a\nDA:1,1\nSF:b\nDA:1,1\nend_of_record\nSF:c\nDA:1,1");
        let paths: Vec<&[u8]> = r.blocks.iter().map(|b| b.path.as_slice()).collect();
        assert_eq!(paths, [b"a" as &[u8], b"b", b"c"]);
        assert!(!r.orphan);
        assert!(r.blocks.iter().all(|b| b.da.len() == 1));
    }

    #[test]
    fn records_outside_a_block_are_orphans() {
        assert!(parse(b"DA:1,1\n").orphan);
        assert!(parse(b"BRDA:1,0,0,1\n").orphan);
        assert!(parse(b"SF:a\nend_of_record\nDA:1,1\n").orphan);
        // Стан блока не переходить між звітами.
        let first = parse(b"SF:a\nDA:1,1\n");
        let second = parse(b"DA:2,1\n");
        assert!(!first.orphan);
        assert!(second.orphan);
        assert!(second.blocks.is_empty());
        // Нерозбірний запис поза блоком — теж без власника.
        assert!(parse(b"DA:x\n").orphan);
        assert!(parse(b"DA\n").orphan);
        assert!(parse(b"BRDA\n").orphan);
    }

    #[test]
    fn defect_stays_with_its_block() {
        let r = parse(b"SF:a\nDA:x,1\nend_of_record\nSF:b\nDA:1,1\nend_of_record\n");
        assert!(!r.orphan);
        assert_eq!(check(&[&r.blocks[0]], 5), Err(Defect::InvalidRecord));
        assert_eq!(check(&[&r.blocks[1]], 5), Ok(()));
    }

    #[test]
    fn malformed_records_are_invalid() {
        for bad in [
            "DA",
            "BRDA",
            "DA:1",
            "DA:1,2,3,4",
            "DA:,1",
            "DA:1,",
            "DA:a,1",
            "DA:1.5,1",
            "DA:+1,1",
            "DA:1,-1",
            "DA:1,x",
            "BRDA:1,0,0",
            "BRDA:1,0,0,1,2",
            "BRDA:1,,0,1",
            "BRDA:1,0,,1",
            "BRDA:1,0,0,",
            "BRDA:1,0,0,-1",
            "BRDA:1,0,0,x",
            "BRDA:x,0,0,1",
        ] {
            let text = format!("SF:a\n{bad}\nend_of_record\n");
            assert_eq!(merged(&[&text], 9), Err(Defect::InvalidRecord), "{bad}");
        }
    }

    #[test]
    fn da_accepts_checksum_and_brda_dash() {
        let text = "SF:a\nDA:1,1,abc==\nDA:2,0,\nBRDA:2,0,0,-\nend_of_record\n";
        assert_eq!(merged(&[text], 2), Ok(counts(2, 1, 1, 1)));
    }

    #[test]
    fn ignored_records_and_bad_totals_do_not_matter() {
        let text =
            "TN:x\nSF:a\nFN:1,f\nFNDA:1,f\nLF:100\nLH:100\nBRF:9\nBRH:9\nDA:1,0\nend_of_record\n";
        assert_eq!(merged(&[text], 3), Ok(counts(1, 1, 0, 0)));
    }

    #[test]
    fn line_bounds_for_both_records() {
        for rec in ["DA:{},1", "BRDA:{},0,0,1"] {
            let at = |n: &str| format!("SF:a\n{}\nend_of_record\n", rec.replace("{}", n));
            assert!(merged(&[&at("1")], 3).is_ok());
            assert!(merged(&[&at("3")], 3).is_ok());
            for n in [
                "0",
                "-1",
                "4",
                "99999999999999999999999",
                "-99999999999999999999999",
            ] {
                assert_eq!(
                    merged(&[&at(n)], 3),
                    Err(Defect::LineOutOfRange),
                    "{rec} {n}"
                );
            }
        }
    }

    // Виявляє зміну пріоритету: нерозбірний запис не повинен поступатися помилці номера рядка.
    #[test]
    fn invalid_record_wins_over_range() {
        let text = "SF:a\nDA:0,1\nSF:a\nDA:1\nend_of_record\n";
        assert_eq!(merged(&[text], 3), Err(Defect::InvalidRecord));
    }

    #[test]
    fn da_counts_and_duplicates_sum_across_reports() {
        let a = "SF:a\nDA:1,0\nDA:1,0\nDA:2,1\nend_of_record\n";
        let b = "SF:a\nDA:1,3\nend_of_record\n";
        assert_eq!(merged(&[a], 2), Ok(counts(2, 1, 0, 0)));
        assert_eq!(merged(&[a, b], 2), Ok(counts(2, 0, 0, 0)));
    }

    // Виявляє переповнення чи обнулення надвеликих лічильників: нуль і додатне не змішуються.
    #[test]
    fn huge_counts_keep_zero_vs_positive() {
        let big = "9".repeat(40);
        let zeros = "0".repeat(40);
        let text = format!(
            "SF:a\nDA:1,{big}\nDA:1,{big}\nDA:2,{zeros}\nBRDA:3,0,0,{big}\nBRDA:3,0,0,{big}\nBRDA:3,0,1,{zeros}\nend_of_record\n"
        );
        assert_eq!(merged(&[&text], 3), Ok(counts(3, 1, 2, 1)));
    }

    #[test]
    fn block_ids_rank_numerically_then_text() {
        let ids: [&[u8]; 6] = [b"x", b"10", b"9", b"010", b"A", b"0"];
        let mut sorted = ids.to_vec();
        sorted.sort_by(|a, b| cmp_block_id(a, b));
        assert_eq!(sorted, [b"0" as &[u8], b"9", b"010", b"10", b"A", b"x"]);
        // Надвеликі числа порівнюються за значенням, а не переповнюються.
        let (big, bigger) = (
            format!("1{}", "0".repeat(30)),
            format!("1{}", "0".repeat(31)),
        );
        assert_eq!(
            cmp_block_id(big.as_bytes(), bigger.as_bytes()),
            Ordering::Less
        );
    }

    #[test]
    fn ranks_are_local_to_line_and_block() {
        // Рядок 1: блок 5 → ранг 0 у кожному SF; рядок 2: блоки 7 і 3 мають ранги 1 і 0
        // незалежно від рядка 1.
        let a = "SF:a\nBRDA:1,5,0,1\nBRDA:2,7,0,0\nBRDA:2,3,0,1\nend_of_record\n";
        let b = "SF:a\nBRDA:1,9,0,0\nBRDA:2,1,0,0\nBRDA:2,8,0,1\nend_of_record\n";
        // Рядок 1: ключ (0,0) покритий у a. Рядок 2: (0,0) покритий у a, (1,0) покритий у b.
        assert_eq!(merged(&[a, b], 2), Ok(counts(2, 0, 3, 0)));
    }

    #[test]
    fn same_number_spellings_do_not_collide() {
        // `10` і `010` — різні ідентифікатори одного SF: різні ранги, тож це дві гілки.
        let text = "SF:a\nBRDA:1,10,0,1\nBRDA:1,010,0,0\nend_of_record\n";
        assert_eq!(merged(&[text], 1), Ok(counts(1, 0, 2, 1)));
    }

    // Виявляє хибний підрахунок непокритої умови, оголошеної лише в частині блоків рядка.
    #[test]
    fn zero_key_counts_only_when_declared_in_every_branch_block() {
        let a = "SF:a\nDA:1,1\nBRDA:1,0,0,1\nBRDA:1,0,1,0\nend_of_record\n";
        let partial = "SF:a\nBRDA:1,0,0,1\nend_of_record\n";
        let full = "SF:a\nBRDA:1,4,0,1\nBRDA:1,4,1,0\nend_of_record\n";
        let da_only = "SF:a\nDA:1,0\nend_of_record\n";
        assert_eq!(merged(&[a], 1), Ok(counts(1, 0, 2, 1)));
        assert_eq!(merged(&[a, partial], 1), Ok(counts(1, 0, 1, 0)));
        assert_eq!(merged(&[a, full], 1), Ok(counts(1, 0, 2, 1)));
        assert_eq!(merged(&[a, da_only], 1), Ok(counts(1, 0, 2, 1)));
    }

    #[test]
    fn covered_key_in_one_block_is_enough() {
        let a = "SF:a\nBRDA:1,0,0,0\nend_of_record\n";
        let b = "SF:a\nBRDA:1,0,0,2\nend_of_record\n";
        assert_eq!(merged(&[a], 1), Ok(counts(1, 1, 1, 1)));
        assert_eq!(merged(&[a, b], 1), Ok(counts(1, 0, 1, 0)));
    }

    #[test]
    fn branch_only_line_counts_even_when_all_keys_dropped() {
        let hit = "SF:a\nBRDA:1,0,0,1\nend_of_record\n";
        let miss = "SF:a\nBRDA:1,0,0,0\nend_of_record\n";
        let other = "SF:a\nBRDA:1,0,1,0\nend_of_record\n";
        assert_eq!(merged(&[hit], 1), Ok(counts(1, 0, 1, 0)));
        assert_eq!(merged(&[miss], 1), Ok(counts(1, 1, 1, 1)));
        assert_eq!(merged(&[miss, other], 1), Ok(counts(1, 1, 0, 0)));
    }

    #[test]
    fn empty_block_is_all_zero() {
        assert_eq!(
            merged(&["SF:a\nend_of_record\n"], 1),
            Ok(counts(0, 0, 0, 0))
        );
    }

    fn fragment(file: &str, name: &str) -> String {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts-coverage");
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
    fn corpus_matches_expected_tsv() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts-coverage");
        let table = std::fs::read_to_string(dir.join("expected.tsv")).unwrap();
        let mut rows = 0;
        for row in table.lines().skip(1) {
            let f: Vec<&str> = row.split('\t').collect();
            assert_eq!(f.len(), 10, "{row}");
            if !f[2].contains('#') || f[2].starts_with(['@', '=']) {
                // Рядок без фрагмента LCOV: джерельний файл перевіряє `ts_coverage`, а сценарії
                // з префіксами `@` і `=` — тести `coverage_report`.
                continue;
            }
            let reports: Vec<Report> = f[2]
                .split(',')
                .map(|input| {
                    let (file, name) = input.split_once('#').unwrap();
                    parse(fragment(file, name).as_bytes())
                })
                .collect();
            let blocks: Vec<&Block> = reports.iter().flat_map(|r| r.blocks.iter()).collect();
            let orphan = reports.iter().any(|r| r.orphan);
            assert_eq!(orphan, f[4] == "yes", "{}: orphan", f[0]);
            let result = merge(&blocks, f[3].parse().unwrap());
            match (f[5], result) {
                ("-", Ok(c)) => {
                    let want: Vec<u64> = f[6..].iter().map(|n| n.parse().unwrap()).collect();
                    let got = [
                        c.lines,
                        c.uncovered_lines,
                        c.conditions,
                        c.uncovered_conditions,
                    ];
                    assert_eq!(got.as_slice(), want.as_slice(), "{}", f[0]);
                }
                ("lcov-invalid-record", Err(Defect::InvalidRecord)) => {}
                ("lcov-line-out-of-range", Err(Defect::LineOutOfRange)) => {}
                (want, got) => panic!("{}: очікувано {want}, отримано {got:?}", f[0]),
            }
            rows += 1;
        }
        assert!(rows > 30);
    }
}
