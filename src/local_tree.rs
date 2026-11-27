//! Відбір і читання локальних файлів `.ts` для аналізу без Git: відбирає за іменем і типом запису,
//! впорядковує ключі й читає вміст окремо. Метрик не рахує.

use std::fs;
use std::path::{Path, PathBuf};

use crate::complexity_report::FileInput;
use crate::git_tree::ReadError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    File,
    Directory,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Selected {
    /// Шлях відносно кореня з роздільником `/` у байтах; для кореня-файла це його ім'я (для
    /// символьного посилання — ім'я самого посилання).
    pub key: Vec<u8>,
    /// Шлях, за яким файл читається з диска.
    pub disk: PathBuf,
}

/// Політика відбору викликача: суфікси файлів (з урахуванням регістру) і теки, які пропускаються
/// лише коли вкладені; на явний корінь ці пропуски не діють.
pub struct Policy {
    pub suffixes: &'static [&'static [u8]],
    pub skip_nested_dirs: &'static [&'static str],
}

pub const METRICS: Policy = Policy {
    suffixes: &[b".ts"],
    skip_nested_dirs: &[],
};

/// Політика `check`: суфікси мов, які підтримують правила (`.ts` і `.rs`), вкладені теки
/// `target` пропускаються.
pub const CHECK: Policy = Policy {
    suffixes: &[b".ts", b".rs"],
    skip_nested_dirs: &["target"],
};

impl Policy {
    fn accepts(&self, key: &[u8]) -> bool {
        self.suffixes.iter().any(|s| key.ends_with(s))
    }

    /// Чи пропускається тека з цим іменем під час обходу вкладених тек.
    fn skips_dir(&self, name: &[u8]) -> bool {
        EXCLUDED.iter().any(|x| x.as_bytes() == name)
            || self.skip_nested_dirs.iter().any(|x| x.as_bytes() == name)
    }

    /// Відбір кореневого ключа (`a/b.ts`): суфікс і жодної виключеної теки серед цілих компонентів
    /// шляху. Збігається з обходом `select` для вкладених файлів.
    // Споживач — відбір gate з Git-дерева та диска (наступні частини).
    #[allow(dead_code)]
    pub fn selects_key(&self, key: &[u8]) -> bool {
        let mut parts = key.split(|b| *b == b'/');
        let _file = parts.next_back();
        self.accepts(key) && !parts.any(|dir| self.skips_dir(dir))
    }

    fn describe(&self) -> String {
        let list: Vec<String> = self
            .suffixes
            .iter()
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        list.join(", ")
    }
}

/// Імена, які обхід пропускає на будь-якій глибині; на сам корінь це не поширюється.
const EXCLUDED: [&str; 2] = [".git", "node_modules"];

#[cfg(unix)]
fn os_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    std::os::unix::ffi::OsStrExt::as_bytes(s).to_vec()
}

#[cfg(not(unix))]
fn os_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

/// Повний відбір без читання вмісту. `Err` — фатальна відмова: повний перелік скласти неможливо.
///
/// Явно вказаний корінь розіменовується, тож символьне посилання на файл чи теку працює як ціль.
/// Під час обходу теки символьні посилання пропускаються, а записи `.git` і `node_modules` не
/// беруться до уваги. Відбираються звичайні файли з суфіксом із політики, зокрема `.d.ts` і `.test.ts`.
/// Результат відсортований за байтами повного ключа, а не за компонентами шляху: `a.b/y.ts` <
/// `a.ts` < `a/z.ts`. Корінь-файл без дозволеного суфікса також дає `Err`.
pub fn select(root: &Path, policy: &Policy) -> Result<(InputKind, Vec<Selected>), String> {
    let meta = fs::metadata(root)
        .map_err(|e| format!("не вдалося визначити тип {}: {e}", root.display()))?;
    if meta.is_file() {
        let name = root
            .file_name()
            .ok_or_else(|| format!("{}: немає імені файла", root.display()))?;
        let key = os_bytes(name);
        if !policy.accepts(&key) {
            return Err(format!(
                "{}: підтримуються лише суфікси {}",
                root.display(),
                policy.describe()
            ));
        }
        return Ok((
            InputKind::File,
            vec![Selected {
                key,
                disk: root.to_path_buf(),
            }],
        ));
    }
    if !meta.is_dir() {
        return Err(format!("{}: не звичайний файл і не тека", root.display()));
    }
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), Vec::<u8>::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .map_err(|e| format!("не вдалося перелічити {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("помилка переліку {}: {e}", dir.display()))?;
            let name = entry.file_name();
            if EXCLUDED.iter().any(|x| name == *x) {
                continue;
            }
            let ty = entry
                .file_type()
                .map_err(|e| format!("не вдалося визначити тип {}: {e}", entry.path().display()))?;
            let mut key = prefix.clone();
            key.extend(os_bytes(&name));
            if ty.is_dir() {
                if policy.skips_dir(&os_bytes(&name)) {
                    continue;
                }
                key.push(b'/');
                stack.push((entry.path(), key));
            } else if ty.is_file() && policy.accepts(&key) {
                found.push(Selected {
                    key,
                    disk: entry.path(),
                });
            }
        }
    }
    found.sort_by(|a, b| a.key.cmp(&b.key));
    Ok((InputKind::Directory, found))
}

/// Відбирає й читає файли; відмова читання зберігається при ключі свого файла.
pub fn collect(root: &Path) -> Result<(InputKind, Vec<FileInput>), String> {
    let (kind, selected) = select(root, &METRICS)?;
    let files = selected
        .into_iter()
        .map(|s| FileInput {
            path: s.key,
            content: fs::read(&s.disk).map_err(|_| ReadError),
        })
        .collect();
    Ok((kind, files))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("osavul-lt-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn touch(root: &Path, rel: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"function f() {}\n").unwrap();
    }

    fn keys(root: &Path) -> Vec<String> {
        keys_with(root, &METRICS)
    }

    fn keys_with(root: &Path, policy: &Policy) -> Vec<String> {
        select(root, policy)
            .unwrap()
            .1
            .into_iter()
            .map(|s| String::from_utf8(s.key).unwrap())
            .collect()
    }

    // Виявляє зміну порядку (за порядком обходу чи за компонентами шляху) та потрапляння
    // виключених тек, тек із суфіксом `.ts` або інших суфіксів у відбір.
    #[test]
    fn selects_sorted_by_whole_path_bytes_and_skips_excluded() {
        let root = tree("matrix");
        for rel in [
            "b.ts",
            "a.ts",
            "a/z.ts",
            "a.b/y.ts",
            "src/x.test.ts",
            "types.d.ts",
            "ign/ored.ts",
            "deep/.git/h.ts",
            "deep/node_modules/n.ts",
            ".git/g.ts",
            "node_modules/m.ts",
            "U.TS",
            "c.tsx",
            "d.mts",
            "real/inner.ts",
            "target/a.ts",
            "x.rs",
            "target/b.rs",
            "deep/target/c.rs",
            "X.RS",
        ] {
            touch(&root, rel);
        }
        fs::create_dir(root.join("dir.ts")).unwrap();
        // "a.b/y.ts" < "a.ts" < "a/z.ts" за байтами повних шляхів ('.' < '/').
        assert_eq!(
            keys(&root),
            [
                "a.b/y.ts",
                "a.ts",
                "a/z.ts",
                "b.ts",
                "ign/ored.ts",
                "real/inner.ts",
                "src/x.test.ts",
                "target/a.ts",
                "types.d.ts"
            ]
        );
        // Політика з обома суфіксами, як у `check` після додавання TypeScript: вкладені `target`
        // пропущені, регістр суфікса значущий.
        let both = Policy {
            suffixes: &[b".ts", b".rs"],
            skip_nested_dirs: CHECK.skip_nested_dirs,
        };
        assert_eq!(
            keys_with(&root, &both),
            [
                "a.b/y.ts",
                "a.ts",
                "a/z.ts",
                "b.ts",
                "ign/ored.ts",
                "real/inner.ts",
                "src/x.test.ts",
                "types.d.ts",
                "x.rs"
            ]
        );
        // Явний корінь `target` не пропускається.
        assert_eq!(keys_with(&root.join("target"), &both), ["a.ts", "b.rs"]);
    }

    // Фізичні посилання потребують POSIX API (на Windows їх створення може вимагати прав).
    #[cfg(unix)]
    #[test]
    fn internal_symlinks_are_skipped() {
        use std::os::unix::fs::symlink;
        let root = tree("links");
        touch(&root, "a.ts");
        touch(&root, "real/inner.ts");
        symlink("a.ts", root.join("link.ts")).unwrap();
        symlink("real", root.join("linkdir")).unwrap();
        assert_eq!(keys(&root), ["a.ts", "real/inner.ts"]);
    }

    #[cfg(unix)]
    // Виявляє втрату розіменування кореня або підміну ключа іменем цілі замість імені посилання.
    #[test]
    fn root_symlinks_are_dereferenced_and_named_by_link() {
        use std::os::unix::fs::symlink;
        let root = tree("rootlink");
        touch(&root, "real/a.ts");
        touch(&root, "plain.txt");
        symlink("real", root.join("dirlink")).unwrap();
        symlink("plain.txt", root.join("bad.ts")).unwrap();
        symlink("real/a.ts", root.join("alias.ts")).unwrap();
        symlink("real/a.ts", root.join("alias.txt")).unwrap();
        assert_eq!(keys(&root.join("dirlink")), ["a.ts"]);
        let (kind, sel) = select(&root.join("alias.ts"), &METRICS).unwrap();
        assert_eq!(
            (kind, sel[0].key.as_slice()),
            (InputKind::File, &b"alias.ts"[..])
        );
        assert!(select(&root.join("alias.txt"), &METRICS).is_err());
        assert_eq!(keys(&root.join("bad.ts")), ["bad.ts"]);
    }

    // Виявляє розбіжність відбору ключів з обходом: цілі компоненти, різна глибина, роль `target`.
    #[test]
    fn key_selection_uses_whole_directory_components() {
        let both = [(&CHECK, true), (&METRICS, false)];
        let sel = |p: &Policy, k: &str| p.selects_key(k.as_bytes());
        for (policy, _) in both {
            assert!(sel(policy, "a.ts") && sel(policy, "types.d.ts"));
            assert!(sel(policy, "my.git/a.ts") && sel(policy, "node_modules2/a.ts"));
            assert!(sel(policy, "x/.github/a.ts"));
            for k in [".git/a.ts", "x/.git/a.ts", "node_modules/a.ts", "a/node_modules/b/c.ts"] {
                assert!(!sel(policy, k), "{k}");
            }
            assert!(!sel(policy, "a.tsx") && !sel(policy, "A.TS"));
        }
        assert!(sel(&CHECK, "x.rs") && !sel(&METRICS, "x.rs"));
        assert!(!sel(&CHECK, "target/a.ts") && !sel(&CHECK, "deep/target/b.rs"));
        assert!(sel(&METRICS, "target/a.ts") && sel(&METRICS, "deep/target/a.ts"));
        assert!(sel(&CHECK, "target.rs") && sel(&CHECK, "mytarget/a.ts"));
    }

    #[test]
    fn missing_root_and_unsupported_file_are_fatal() {
        let root = tree("fatal");
        touch(&root, "a.txt");
        for policy in [&METRICS, &CHECK] {
            assert!(select(&root.join("none.ts"), policy).is_err());
            assert!(select(&root.join("a.txt"), policy).is_err());
        }
        touch(&root, "r.rs");
        assert!(select(&root.join("r.rs"), &METRICS).is_err());
        assert!(select(&root.join("r.rs"), &CHECK).is_ok());
    }
}
