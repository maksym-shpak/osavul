
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
    pub key: Vec<u8>,
    pub disk: PathBuf,
}

const SUFFIX: &[u8] = b".ts";
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
pub fn select(root: &Path) -> Result<(InputKind, Vec<Selected>), String> {
    // Явно вказаний корінь розіменовується, на відміну від записів усередині теки.
    let meta = fs::metadata(root).map_err(|e| format!("не вдалося визначити тип {}: {e}", root.display()))?;
    if meta.is_file() {
        let name = root
            .file_name()
            .ok_or_else(|| format!("{}: немає імені файла", root.display()))?;
        let key = os_bytes(name);
        if !key.ends_with(SUFFIX) {
            return Err(format!("{}: підтримується лише суфікс .ts", root.display()));
        }
        return Ok((InputKind::File, vec![Selected { key, disk: root.to_path_buf() }]));
    }
    if !meta.is_dir() {
        return Err(format!("{}: не звичайний файл і не тека", root.display()));
    }
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), Vec::<u8>::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| format!("не вдалося перелічити {}: {e}", dir.display()))?;
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
                key.push(b'/');
                stack.push((entry.path(), key));
            } else if ty.is_file() && key.ends_with(SUFFIX) {
                found.push(Selected { key, disk: entry.path() });
            }
        }
    }
    found.sort_by(|a, b| a.key.cmp(&b.key));
    Ok((InputKind::Directory, found))
}

/// Відбирає й читає файли; відмова читання зберігається при ключі свого файла.
pub fn collect(root: &Path) -> Result<(InputKind, Vec<FileInput>), String> {
    let (kind, selected) = select(root)?;
    let files = selected
        .into_iter()
        .map(|s| FileInput { path: s.key, content: fs::read(&s.disk).map_err(|_| ReadError) })
        .collect();
    Ok((kind, files))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

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
        select(root).unwrap().1.into_iter().map(|s| String::from_utf8(s.key).unwrap()).collect()
    }

    #[test]
    fn selects_sorted_by_whole_path_bytes_and_skips_excluded_and_links() {
        let root = tree("matrix");
        for rel in [
            "b.ts", "a.ts", "a/z.ts", "a.b/y.ts", "src/x.test.ts", "types.d.ts", "ign/ored.ts",
            "deep/.git/h.ts", "deep/node_modules/n.ts", ".git/g.ts", "node_modules/m.ts",
            "U.TS", "c.tsx", "d.mts", "real/inner.ts",
        ] {
            touch(&root, rel);
        }
        symlink("a.ts", root.join("link.ts")).unwrap();
        symlink("real", root.join("linkdir")).unwrap();
        fs::create_dir(root.join("dir.ts")).unwrap();
        // "a.b/y.ts" < "a.ts" < "a/z.ts" за байтами повних шляхів ('.' < '/').
        assert_eq!(
            keys(&root),
            [
                "a.b/y.ts", "a.ts", "a/z.ts", "b.ts", "ign/ored.ts", "real/inner.ts", "src/x.test.ts",
                "types.d.ts"
            ]
        );
    }

    #[test]
    fn root_symlinks_are_dereferenced_and_named_by_link() {
        let root = tree("rootlink");
        touch(&root, "real/a.ts");
        touch(&root, "plain.txt");
        symlink("real", root.join("dirlink")).unwrap();
        symlink("plain.txt", root.join("bad.ts")).unwrap();
        symlink("real/a.ts", root.join("alias.ts")).unwrap();
        symlink("real/a.ts", root.join("alias.txt")).unwrap();
        assert_eq!(keys(&root.join("dirlink")), ["a.ts"]);
        let (kind, sel) = select(&root.join("alias.ts")).unwrap();
        assert_eq!((kind, sel[0].key.as_slice()), (InputKind::File, &b"alias.ts"[..]));
        assert!(select(&root.join("alias.txt")).is_err());
        assert_eq!(keys(&root.join("bad.ts")), ["bad.ts"]);
    }

    #[test]
    fn missing_root_and_unsupported_file_are_fatal() {
        let root = tree("fatal");
        touch(&root, "a.txt");
        assert!(select(&root.join("none.ts")).is_err());
        assert!(select(&root.join("a.txt")).is_err());
    }
}
