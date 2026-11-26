//! Зіставлення шляхів `SF` зі звіту LCOV з повним інвентарем `.ts` і явними тестовими виключеннями.
//!
//! Усе зіставлення лексичне: файлів на диску ніхто не відкриває, тож ключі старого коміту працюють без checkout.

use std::collections::HashMap;
use std::ffi::OsStr;

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
                return Err(format!("--tests: шлях має бути відносним: {}", arg.to_string_lossy()));
            }
            let comps = normalize(&[], &bytes, true);
            if comps.first().is_some_and(|c| c.as_slice() == b"..") {
                return Err(format!("--tests: шлях виходить за корінь: {}", arg.to_string_lossy()));
            }
            Ok(comps.join(&b'/'))
        })
        .collect()
}

impl Matcher {
    /// `root` — абсолютна канонічна тека, `keys` — повний інвентар у його порядку, `tests` — результат `parse_tests`.
    pub fn new<'a>(root: &[u8], keys: impl IntoIterator<Item = &'a [u8]>, tests: Vec<Vec<u8>>) -> Self {
        let index = keys.into_iter().enumerate().map(|(i, k)| (k.to_vec(), i)).collect();
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
}
