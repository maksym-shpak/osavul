//! Методика `ts-duplication` версії 1: підготовка рядків токенів файла TypeScript для
//! пошуку точних повторів. Зміна правил нормалізації чи вилучень потребує нової версії.

use crate::ts_source::{ScoreError, line_of, line_starts, with_parsed};
use oxc_ast::ast::{
    CallExpression, Expression, ImportDeclaration, TSImportEqualsDeclaration, VariableDeclaration,
};
use oxc_ast_visit::{Visit, walk};

/// Текст, яким замінюється будь-який рядковий або шаблонний токен із початковою лапкою.
const LITERAL: &str = "LITERAL";

/// Рядок токенів: усі збережені токени з початком на одному фізичному рядку.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Нормалізовані тексти токенів у порядку джерела; їх кількість — вага рядка.
    pub tokens: Vec<String>,
    /// Фізичний рядок (з нуля) початку першого токена.
    pub start_line: usize,
    /// Останній фізичний рядок (з нуля), зайнятий токенами рядка.
    pub end_line: usize,
}

/// Підготовлені дані успішно розібраного файла.
#[derive(Debug, PartialEq, Eq)]
pub struct Prepared {
    /// Кількість фізичних рядків: кінці рядка плюс один.
    pub lines: u64,
    /// Рядки токенів після вилучень і скорочення серій.
    pub rows: Vec<Row>,
}

/// Готує файл до пошуку; шлях лише обирає режим `.d.ts`.
pub fn prepare(path: &[u8], content: &[u8]) -> Result<Prepared, ScoreError> {
    with_parsed(path, content, |text, program, tokens| {
        let mut finder = Removals::default();
        finder.visit_program(program);
        let starts = line_starts(text);
        let mut rows: Vec<Row> = Vec::new();
        let mut current_start: Option<usize> = None;
        for token in tokens {
            let (start, end) = (token.start() as usize, token.end() as usize);
            if end <= start || finder.covers(start) {
                continue;
            }
            let first = line_of(&starts, start);
            let last = line_of(&starts, end - 1);
            let raw = &text[start..end];
            let normalized = if raw.starts_with(['"', '\'', '`']) {
                LITERAL
            } else {
                raw
            };
            if current_start == Some(first) {
                let row = rows.last_mut().expect("row exists");
                row.tokens.push(normalized.to_owned());
                row.end_line = row.end_line.max(last);
            } else {
                current_start = Some(first);
                rows.push(Row {
                    tokens: vec![normalized.to_owned()],
                    start_line: first,
                    end_line: last,
                });
            }
        }
        Prepared {
            lines: starts.len() as u64,
            rows: collapse_runs(rows),
        }
    })
}

/// У серії рівних сусідніх рядків лишає перший і останній; одиничний рядок — один раз.
fn collapse_runs(rows: Vec<Row>) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::with_capacity(rows.len());
    let mut i = 0;
    while i < rows.len() {
        let mut j = i;
        while j + 1 < rows.len() && rows[j + 1].tokens == rows[i].tokens {
            j += 1;
        }
        out.push(rows[i].clone());
        if j > i {
            out.push(rows[j].clone());
        }
        i = j + 1;
    }
    out
}

/// Мінімальна кількість рядків токенів у дублі.
const MIN_ROWS: usize = 10;
/// Мінімальна сумарна кількість токенів у дублі.
const MIN_TOKENS: usize = 100;

/// Входження групи: файл (індекс у вхідному зрізі) і включні фізичні рядки з нуля.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Occurrence {
    pub file: usize,
    pub start_line: usize,
    pub end_line: usize,
}

/// Усі точні входження однієї максимальної послідовності, впорядковані за файлом і початком.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub occurrences: Vec<Occurrence>,
}

/// Кандидат — інтервал `lb..rb` суфіксного масиву зі спільним префіксом довжиною `len` рядків.
/// Старти не копіюються: на періодичних входах вкладені інтервали разом мали б квадратний обсяг.
struct Candidate {
    len: usize,
    lb: usize,
    rb: usize,
}

/// Шукає групи клонів між усіма файлами й усередині кожного.
pub fn find_groups(files: &[&Prepared]) -> Vec<Group> {
    find_groups_with(files, std::collections::hash_map::RandomState::new())
}

/// Те саме з вибором хешера: тести підставляють хешер із постійним хешем, щоб довести, що
/// рівність підтверджується точними ключами, а не хешами.
fn find_groups_with<S: std::hash::BuildHasher>(files: &[&Prepared], hasher: S) -> Vec<Group> {
    // Інтернування рядків за точною послідовністю токенів.
    let mut ids: std::collections::HashMap<&[String], u32, S> =
        std::collections::HashMap::with_hasher(hasher);
    let mut seq: Vec<u32> = Vec::new();
    let mut weight: Vec<usize> = vec![0];
    // Для кожної глобальної позиції: (файл, рядок); для роздільника — рядок = usize::MAX.
    let mut origin: Vec<(usize, usize)> = Vec::new();
    let mut pending: Vec<(usize, usize, u32)> = Vec::new();
    for (f, file) in files.iter().enumerate() {
        for (r, row) in file.rows.iter().enumerate() {
            let next = ids.len() as u32;
            let id = *ids.entry(row.tokens.as_slice()).or_insert(next);
            pending.push((f, r, id));
        }
        pending.push((f, usize::MAX, u32::MAX));
    }
    // Роздільники мають унікальні значення поза алфавітом рядків.
    let alphabet = ids.len() as u32;
    for (f, r, id) in pending {
        if r == usize::MAX {
            seq.push(alphabet + f as u32);
            weight.push(*weight.last().unwrap());
        } else {
            seq.push(id);
            weight.push(*weight.last().unwrap() + files[f].rows[r].tokens.len());
        }
        origin.push((f, r));
    }
    drop(ids);

    let sa = suffix_array(&seq);
    let lcp = lcp_array(&seq, &sa);
    let n = seq.len();

    let left_differs = |starts: &[usize]| -> bool {
        let left = |p: usize| -> Option<u32> {
            if origin[p].1 == 0 { None } else { Some(seq[p - 1]) }
        };
        let first = left(starts[0]);
        first.is_none() || starts.iter().any(|&p| left(p) != first)
    };

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
    for i in 1..=n {
        let cur = if i < n { lcp[i] } else { 0 };
        let mut lb = i - 1;
        while let Some(&(l, b)) = stack.last() {
            if l <= cur {
                break;
            }
            stack.pop();
            lb = b;
            if l >= MIN_ROWS
                && weight[sa[b] + l] - weight[sa[b]] >= MIN_TOKENS
                && left_differs(&sa[b..i])
            {
                candidates.push(Candidate { len: l, lb: b, rb: i });
            }
        }
        if stack.last().is_none_or(|t| t.0 < cur) {
            stack.push((cur, lb));
        }
    }

    let mut rank = vec![0usize; n];
    for (i, &p) in sa.iter().enumerate() {
        rank[p] = i;
    }
    let dropped = drop_contained(&candidates, &sa, &rank, &origin);
    let mut groups: Vec<Group> = candidates
        .iter()
        .zip(dropped)
        .filter(|(_, d)| !d)
        .map(|(c, _)| {
            let mut occurrences: Vec<Occurrence> = sa[c.lb..c.rb]
                .iter()
                .map(|&p| {
                    let (file, s) = origin[p];
                    Occurrence {
                        file,
                        start_line: files[file].rows[s].start_line,
                        end_line: files[file].rows[s + c.len - 1].end_line,
                    }
                })
                .collect();
            occurrences.sort();
            Group { occurrences }
        })
        .collect();
    groups.sort_by(|a, b| a.occurrences.cmp(&b.occurrences));
    groups
}

const NONE: usize = usize::MAX;

/// Позначає групи, усі входження яких покриті входженнями однієї довшої групи з тим самим
/// набором файлів. Інтервали утворюють лам'яну родину (дерево), тому групи, що мають
/// входження на заданій позиції, — це предки листка цієї позиції; старти не розгортаються.
fn drop_contained(
    candidates: &[Candidate],
    sa: &[usize],
    rank: &[usize],
    origin: &[(usize, usize)],
) -> Vec<bool> {
    let n = sa.len();
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by_key(|&c| (candidates[c].lb, std::cmp::Reverse(candidates[c].rb)));
    let mut parent = vec![NONE; candidates.len()];
    let mut deepest = vec![NONE; n];
    let mut open: Vec<usize> = Vec::new();
    let mut next = 0;
    for (r, slot) in deepest.iter_mut().enumerate() {
        while open.last().is_some_and(|&c| candidates[c].rb <= r) {
            open.pop();
        }
        while next < order.len() && candidates[order[next]].lb == r {
            let c = order[next];
            parent[c] = open.last().copied().unwrap_or(NONE);
            open.push(c);
            next += 1;
        }
        *slot = open.last().copied().unwrap_or(NONE);
    }

    let file_set = |c: &Candidate| -> Vec<usize> {
        let mut v: Vec<usize> = sa[c.lb..c.rb].iter().map(|&p| origin[p].0).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    // Кожне входження `g` лежить у якомусь входженні `h`: старт `h` — на `d` рядків лівіше.
    let covers = |h: &Candidate, g: &Candidate| -> bool {
        sa[g.lb..g.rb].iter().all(|&q| {
            let reach = origin[q].1.min(h.len - g.len);
            (0..=reach).any(|d| (h.lb..h.rb).contains(&rank[q - d]))
        })
    };

    let mut dropped = vec![false; candidates.len()];
    let mut stamp = vec![NONE; candidates.len()];
    for (g, gc) in candidates.iter().enumerate() {
        let t0 = sa[gc.lb];
        let mut g_set: Option<Vec<usize>> = None;
        'search: for d in 0..=origin[t0].1 {
            let mut h = deepest[rank[t0 - d]];
            while h != NONE {
                let hc = &candidates[h];
                if hc.len < gc.len + d {
                    break;
                }
                if hc.len > gc.len && stamp[h] != g {
                    stamp[h] = g;
                    let gs = g_set.get_or_insert_with(|| file_set(gc));
                    if file_set(hc) == *gs && covers(hc, gc) {
                        dropped[g] = true;
                        break 'search;
                    }
                }
                h = parent[h];
            }
        }
    }
    dropped
}

/// Суфіксний масив подвоєнням префіксів.
fn suffix_array(seq: &[u32]) -> Vec<usize> {
    let n = seq.len();
    if n == 0 {
        return Vec::new();
    }
    let mut sa: Vec<usize> = (0..n).collect();
    let mut rank: Vec<usize> = seq.iter().map(|&x| x as usize).collect();
    let mut tmp = vec![0usize; n];
    let mut k = 1;
    loop {
        let key = |i: usize| (rank[i], rank.get(i + k).map_or(0, |r| r + 1));
        sa.sort_unstable_by_key(|&i| key(i));
        tmp[sa[0]] = 0;
        for w in 1..n {
            tmp[sa[w]] = tmp[sa[w - 1]] + usize::from(key(sa[w]) != key(sa[w - 1]));
        }
        rank.copy_from_slice(&tmp);
        if rank[sa[n - 1]] == n - 1 {
            break;
        }
        k *= 2;
    }
    sa
}

/// Довжини спільних префіксів сусідніх суфіксів (Kasai); `lcp[i]` — для `sa[i-1]` і `sa[i]`.
fn lcp_array(seq: &[u32], sa: &[usize]) -> Vec<usize> {
    let n = seq.len();
    let mut rank = vec![0usize; n];
    for (i, &p) in sa.iter().enumerate() {
        rank[p] = i;
    }
    let mut lcp = vec![0usize; n];
    let mut h = 0usize;
    for p in 0..n {
        if rank[p] > 0 {
            let q = sa[rank[p] - 1];
            while p + h < n && q + h < n && seq[p + h] == seq[q + h] {
                h += 1;
            }
            lcp[rank[p]] = h;
            h = h.saturating_sub(1);
        } else {
            h = 0;
        }
    }
    lcp
}

/// Байтові діапазони декларацій, чиї токени вилучаються.
#[derive(Default)]
struct Removals {
    ranges: Vec<(usize, usize)>,
}

impl Removals {
    fn covers(&self, offset: usize) -> bool {
        // Діапазони не перекриваються: внутрішність вилученої декларації не обходиться.
        let i = self.ranges.partition_point(|&(s, _)| s <= offset);
        i > 0 && offset < self.ranges[i - 1].1
    }
}

impl<'a> Visit<'a> for Removals {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        self.ranges
            .push((it.span.start as usize, it.span.end as usize));
    }

    fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
        self.ranges
            .push((it.span.start as usize, it.span.end as usize));
    }

    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        if let [declarator] = it.declarations.as_slice()
            && declarator.init.as_ref().is_some_and(is_require_form)
        {
            self.ranges
                .push((it.span.start as usize, it.span.end as usize));
            return;
        }
        walk::walk_variable_declaration(self, it);
    }
}

/// `require(...)` — виклик ідентифікатора `require` (звичайні дужки прозорі).
fn is_require_call(expr: &Expression<'_>) -> bool {
    match expr.without_parentheses() {
        Expression::CallExpression(call) => is_plain_require(call),
        _ => false,
    }
}

fn is_plain_require(call: &CallExpression<'_>) -> bool {
    matches!(
        call.callee.without_parentheses(),
        Expression::Identifier(id) if id.name == "require"
    )
}

/// Одна з чотирьох форм ініціалізатора: `require()`, `require()()`, `require().p`
/// (також `require()[p]`), `require().m()`.
fn is_require_form(init: &Expression<'_>) -> bool {
    match init.without_parentheses() {
        Expression::CallExpression(call) => {
            is_plain_require(call) || {
                let callee = call.callee.without_parentheses();
                is_require_call(callee) || is_member_of_require(callee)
            }
        }
        other => is_member_of_require(other),
    }
}

fn is_member_of_require(expr: &Expression<'_>) -> bool {
    match expr.without_parentheses() {
        Expression::StaticMemberExpression(m) => is_require_call(&m.object),
        Expression::ComputedMemberExpression(m) => is_require_call(&m.object),
        Expression::PrivateFieldExpression(m) => is_require_call(&m.object),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(src: &str) -> Vec<Vec<String>> {
        prepare(b"a.ts", src.as_bytes())
            .unwrap()
            .rows
            .into_iter()
            .map(|r| r.tokens)
            .collect()
    }

    /// Усі збережені токени через пробіл.
    fn flat(src: &str) -> String {
        rows(src).concat().join(" ")
    }

    fn coords(src: &str) -> Vec<(usize, usize)> {
        prepare(b"a.ts", src.as_bytes())
            .unwrap()
            .rows
            .iter()
            .map(|r| (r.start_line, r.end_line))
            .collect()
    }

    #[test]
    fn template_parts_follow_oxc_token_boundaries() {
        // Голова — один LITERAL; середина й хвіст починаються з `}` і лишаються дослівними.
        assert_eq!(
            rows("a(`a${b}c${d}e`);"),
            [["a", "(", "LITERAL", "b", "}c${", "d", "}e`", ")", ";"]]
        );
        // Вкладений шаблон: голова внутрішнього — LITERAL, його хвіст `}`` дослівний.
        assert_eq!(
            rows("`x${`y${z}`}w`;"),
            [["LITERAL", "LITERAL", "z", "}`", "}w`", ";"]]
        );
        // Вираз усередині підстановки нормалізується звичайно.
        assert_eq!(flat("`a${'s'}b`;"), "LITERAL LITERAL }b` ;");
        // Шаблон без підстановок — один токен.
        assert_eq!(flat("`a`;"), "LITERAL ;");
    }

    #[test]
    fn multiline_last_token_keeps_end_coordinate() {
        // Токен після багаторядкового починає рядок, де той закінчується.
        assert_eq!(
            coords("let a = `x\ny\nz`;\nlet b;\n"),
            [(0, 2), (2, 2), (3, 3)]
        );
        assert_eq!(
            rows("let a = `x\ny\nz` + 1;"),
            vec![vec!["let", "a", "=", "LITERAL"], vec!["+", "1", ";"]]
        );
    }

    #[test]
    fn quotes_become_literal_and_equal_the_identifier() {
        assert_eq!(
            flat("a(\"x\", 'y', `z`, LITERAL);"),
            "a ( LITERAL , LITERAL , LITERAL , LITERAL ) ;"
        );
        assert_eq!(flat("f(\"a\");"), flat("f(LITERAL);"));
    }

    #[test]
    fn other_tokens_are_verbatim() {
        assert_eq!(
            flat("const n = 0x10 + 1_0; if (a) /re+/g.test(a);"),
            "const n = 0x10 + 1_0 ; if ( a ) /re+/g . test ( a ) ;"
        );
        assert_eq!(flat("a1 = 1; a2 = 1;"), "a1 = 1 ; a2 = 1 ;");
        assert_ne!(flat("let a = 1;"), flat("let b = 1;"));
        assert_ne!(flat("let a = 1.0;"), flat("let a = 1;"));
    }

    #[test]
    fn comments_whitespace_and_shebang_add_nothing() {
        assert_eq!(
            rows("#!/usr/bin/env node\n/* c */ let /* d */ a = 1; // e\n\n\n"),
            [["let", "a", "=", "1", ";"]]
        );
        assert_eq!(rows("\u{feff}// c\n"), Vec::<Vec<String>>::new());
        assert_eq!(rows(""), Vec::<Vec<String>>::new());
    }

    #[test]
    fn static_imports_are_removed() {
        for src in [
            "import a from 'a';\nlet x;",
            "import { a, b as c } from 'a';\nlet x;",
            "import * as a from 'a';\nlet x;",
            "import 'a';\nlet x;",
            "import type { A } from 'a';\nlet x;",
            "import type A from 'a'\nlet x;",
            "import a = require('a');\nlet x;",
            "import a = B.C;\nlet x;",
            "import a = B.C; let x;",
        ] {
            assert_eq!(flat(src), "let x ;", "{src}");
        }
    }

    #[test]
    fn kept_import_like_forms() {
        assert_eq!(
            flat("export { a } from 'a';"),
            "export { a } from LITERAL ;"
        );
        assert_eq!(flat("export * from 'a';"), "export * from LITERAL ;");
        assert_eq!(flat("import('a');"), "import ( LITERAL ) ;");
        assert_eq!(
            flat("let t: import('a').T;"),
            "let t : import ( LITERAL ) . T ;"
        );
        // Зовнішній `export` декларації `import =` лишається.
        assert!(flat("export import a = B.C;").starts_with("export"));
    }

    #[test]
    fn require_declarations_are_removed() {
        for init in [
            "require('a')",
            "(require('a'))",
            "require('a')('b')",
            "(require('a'))('b')",
            "require('a').p",
            "require('a')['p']",
            "(require('a')).p",
            "require('a').m()",
            "require('a')['m']()",
            "(require('a').m)()",
        ] {
            for kw in ["const", "let", "var"] {
                let src = format!("{kw} v = {init};\nlet x;");
                assert_eq!(flat(&src), "let x ;", "{src}");
            }
        }
        // Destructuring і анотація типу — теж оголошення з одним декларатором.
        assert_eq!(flat("const { a } = require('a');\nlet x;"), "let x ;");
        assert_eq!(flat("const v: T = require('a');\nlet x;"), "let x ;");
    }

    #[test]
    fn require_private_member_declarations_are_removed() {
        for init in ["require('a').#m", "require('a').#m()", "(require('a')).#m"] {
            let src = format!("class C {{ #m; f() {{ const v = {init};\nlet x; }} }}");
            assert!(!flat(&src).contains("require"), "{src}");
            assert!(flat(&src).contains("let x"), "{src}");
        }
    }

    #[test]
    fn require_like_forms_are_kept() {
        for src in [
            "const a = require('a'), b = 1;",
            "const a = 1, b = require('a');",
            "const a = require('a') as T;",
            "const a = <T>require('a');",
            "const a = require('a')!;",
            "const a = require('a')?.p;",
            "const a = require('a').p.q;",
            "const a = require('a').m().n;",
            "const a = require('a').m().n();",
            "const a = require('a') + 1;",
            "const a = foo.require('a');",
            "const a = require;",
            "const a = await import('a');",
            "const a = require?.('a');",
        ] {
            assert!(flat(src).starts_with("const a"), "{src}");
            assert!(flat(src).ends_with(';'), "{src}");
        }
    }

    #[test]
    fn removal_stops_at_declaration_bounds() {
        // Крапка з комою належить декларації; сусідній код на рядку лишається.
        assert_eq!(flat("const a = require('a'); let x;"), "let x ;");
        assert_eq!(flat("let x; const a = require('a');"), "let x ;");
        assert_eq!(flat("export const a = require('a');"), "export");
        assert_eq!(flat("export const a = require('a')\nlet x"), "export let x");
        // Заголовок циклу: розділювачі лишаються.
        assert_eq!(
            flat("for (const a = require('a'); i < 1; i++) {}"),
            "for ( ; i < 1 ; i ++ ) { }"
        );
        // Вкладена декларація теж вилучається.
        assert_eq!(
            flat("function f() { const a = require('a'); return 1; }"),
            "function f ( ) { return 1 ; }"
        );
    }

    #[test]
    fn removed_lines_leave_no_empty_row() {
        assert_eq!(
            rows("import a from 'a';\nimport b from 'b';\nlet x;"),
            [["let", "x", ";"]]
        );
        assert_eq!(coords("import a from 'a';\nlet x;"), [(1, 1)]);
    }

    #[test]
    fn single_row_stays_once_and_runs_keep_both_ends() {
        assert_eq!(coords("a;\nb;\n"), [(0, 0), (1, 1)]);
        assert_eq!(coords("a;\na;\n"), [(0, 0), (1, 1)]);
        assert_eq!(
            coords("a;\na;\na;\na;\nb;\na;\n"),
            [(0, 0), (3, 3), (4, 4), (5, 5)]
        );
        // Рівні рядки розділені вилученим рядком лишаються сусідніми й скорочуються.
        assert_eq!(
            coords("a;\nimport x from 'x';\na;\na;\nb;"),
            [(0, 0), (3, 3), (4, 4)]
        );
        // Рядок із різними токенами серію не продовжує.
        assert_eq!(coords("a;\na; a;\na;\n"), [(0, 0), (1, 1), (2, 2)]);
    }

    #[test]
    fn lines_count_all_five_terminators_and_bom() {
        for t in ["\n", "\r", "\r\n", "\u{2028}", "\u{2029}"] {
            let p = prepare(b"a.ts", format!("a;{t}b;{t}").as_bytes()).unwrap();
            assert_eq!(p.lines, 3, "{t:?}");
            assert_eq!(
                p.rows
                    .iter()
                    .map(|r| (r.start_line, r.end_line))
                    .collect::<Vec<_>>(),
                [(0, 0), (1, 1)],
                "{t:?}"
            );
            let m = prepare(b"a.ts", format!("let a = `x{t}y`;{t}b;").as_bytes()).unwrap();
            assert_eq!(m.rows[0].end_line, 1, "{t:?}");
            assert_eq!(m.rows[1].start_line, 1, "{t:?}");
            assert_eq!(m.rows[2].start_line, 2, "{t:?}");
        }
        assert_eq!(prepare(b"a.ts", b"").unwrap().lines, 1);
        let bom = prepare(b"a.ts", "\u{feff}a;\nb;".as_bytes()).unwrap();
        assert_eq!((bom.rows[0].start_line, bom.rows[1].start_line), (0, 1));
        assert_eq!(bom.rows[0].tokens, ["a", ";"]);
    }

    #[test]
    fn d_ts_mode_and_diagnostics() {
        assert!(prepare(b"a.d.ts", b"declare function f(): void;\n").is_ok());
        assert_eq!(
            prepare(b"a.d.ts", b"let a = ;\n").map(|p| p.lines),
            Err(ScoreError::Syntax)
        );
        assert_eq!(prepare(b"a.ts", b"let a = ;\n"), Err(ScoreError::Syntax));
        assert_eq!(prepare(b"a.ts", b"a;\xff"), Err(ScoreError::InvalidUtf8));
    }

    mod groups {
        use super::super::*;
        use std::hash::{BuildHasher, Hasher};

        /// Файл із рядків `(ідентифікатор, кількість токенів)`; рядок i лежить на фізичному рядку i.
        fn file(rows: &[(&str, usize)]) -> Prepared {
            Prepared {
                lines: rows.len() as u64 + 1,
                rows: rows
                    .iter()
                    .enumerate()
                    .map(|(i, (id, n))| Row {
                        tokens: vec![(*id).to_owned(); *n],
                        start_line: i,
                        end_line: i,
                    })
                    .collect(),
            }
        }

        /// Блок з `count` унікальних рядків по 10 токенів з префіксом імені.
        fn block(prefix: &str, count: usize) -> Vec<(String, usize)> {
            (0..count).map(|i| (format!("{prefix}{i}"), 10)).collect()
        }

        fn make(parts: &[&[(String, usize)]]) -> Prepared {
            let rows: Vec<(&str, usize)> = parts
                .iter()
                .flat_map(|p| p.iter().map(|(s, n)| (s.as_str(), *n)))
                .collect();
            file(&rows)
        }

        fn occs(groups: &[Group]) -> Vec<Vec<(usize, usize, usize)>> {
            groups
                .iter()
                .map(|g| {
                    g.occurrences
                        .iter()
                        .map(|o| (o.file, o.start_line, o.end_line))
                        .collect()
                })
                .collect()
        }

        fn run(files: &[Prepared]) -> Vec<Group> {
            let refs: Vec<&Prepared> = files.iter().collect();
            find_groups(&refs)
        }

        #[test]
        fn thresholds_are_inclusive_on_rows_and_tokens() {
            let s10 = block("s", 10);
            let pair = |a: &[(String, usize)]| vec![make(&[a]), make(&[a])];
            assert_eq!(occs(&run(&pair(&s10))), [[(0, 0, 9), (1, 0, 9)]]);
            // 9 рядків по 12 токенів: токенів досить, рядків ні.
            let s9: Vec<_> = (0..9).map(|i| (format!("s{i}"), 12)).collect();
            assert!(run(&pair(&s9)).is_empty());
            // 10 рядків: 99 токенів — ні, 100 — так.
            let mut t99 = block("s", 10);
            t99[0].1 = 9;
            assert!(run(&pair(&t99)).is_empty());
            t99[0].1 = 10;
            assert_eq!(run(&pair(&t99)).len(), 1);
        }

        struct ConstState;
        struct ConstHasher;
        impl Hasher for ConstHasher {
            fn finish(&self) -> u64 {
                0
            }
            fn write(&mut self, _: &[u8]) {}
        }
        impl BuildHasher for ConstState {
            type Hasher = ConstHasher;
            fn build_hasher(&self) -> ConstHasher {
                ConstHasher
            }
        }

        #[test]
        fn forced_hash_collision_keeps_exact_equality() {
            let a = block("s", 10);
            let b = block("t", 10);
            let files = [make(&[&a]), make(&[&b]), make(&[&a])];
            let refs: Vec<&Prepared> = files.iter().collect();
            let groups = find_groups_with(&refs, ConstState);
            assert_eq!(occs(&groups), [[(0, 0, 9), (2, 0, 9)]]);
            let files = [make(&[&a]), make(&[&b])];
            let refs: Vec<&Prepared> = files.iter().collect();
            assert!(find_groups_with(&refs, ConstState).is_empty());
        }

        #[test]
        fn extends_both_ways_and_stops_at_file_boundary() {
            let (x, s, y, z) = (block("x", 3), block("s", 10), block("y", 3), block("z", 3));
            // Спільне ядро з різними контекстами: рівно 10 рядків.
            let files = [make(&[&x, &s, &y]), make(&[&z, &s, &x])];
            assert_eq!(occs(&run(&files)), [[(0, 3, 12), (1, 3, 12)]]);
            // Продовження ліворуч до початку файла: група охоплює і спільний префікс.
            let files = [make(&[&s, &y]), make(&[&x, &s, &z])];
            assert_eq!(occs(&run(&files)), [[(0, 0, 9), (1, 3, 12)]]);
        }

        #[test]
        fn group_has_every_start_including_overlaps() {
            let (p, f, g) = (block("p", 10), block("f", 1), block("g", 1));
            // Три входження з різними контекстами лишаються в одній групі.
            let groups = run(&[make(&[&p, &f, &p, &g, &p])]);
            assert_eq!(occs(&groups), [[(0, 0, 9), (0, 11, 20), (0, 22, 31)]]);
            // Перекривні старти: триразовий вплив поглинається довшою групою з двох входжень.
            let groups = run(&[make(&[&p, &p, &p])]);
            assert_eq!(occs(&groups), [[(0, 0, 19), (0, 10, 29)]]);
        }

        #[test]
        fn dense_repeat_collapses_into_one_longest_group() {
            let p = block("p", 10);
            let f = make(&[&p, &p, &p, &p, &p, &p]);
            assert_eq!(occs(&run(&[f])), [[(0, 0, 49), (0, 10, 59)]]);
        }

        #[test]
        fn periodic_input_yields_one_group() {
            let rows: Vec<(&str, usize)> = (0..1500)
                .map(|i| if i % 2 == 0 { ("a", 10) } else { ("b", 10) })
                .collect();
            let groups = run(&[file(&rows)]);
            assert_eq!(occs(&groups), [[(0, 0, 1497), (0, 2, 1499)]]);
        }

        #[test]
        fn contained_group_is_dropped_only_when_all_occurrences_are_covered() {
            let (s, q, f1, f2, f3) = (
                block("s", 10),
                block("q", 1),
                block("f", 9),
                block("g", 9),
                block("h", 9),
            );
            let l: Vec<(String, usize)> = [s.clone(), q.clone(), s.clone()].concat();
            let a = make(&[&l, &f1, &l]);
            // Усі чотири входження S лежать у двох копіях L: S відкинуто.
            let groups = run(&[make(&[&l, &f1, &l])]);
            assert_eq!(occs(&groups), [[(0, 0, 20), (0, 30, 50)]]);
            // Інший набір файлів зберігає S.
            let groups = run(&[a.clone_prepared(), make(&[&s])]);
            assert_eq!(groups.len(), 2);
            // Непокрите входження зберігає S.
            let groups = run(&[make(&[&l, &f1, &l, &f2, &s, &f3])]);
            assert_eq!(groups.len(), 2);
        }

        #[test]
        fn coverage_by_several_groups_does_not_absorb() {
            let (p, s, q, fa, fb) = (
                block("p", 10),
                block("s", 10),
                block("q", 10),
                block("a", 9),
                block("b", 9),
            );
            let files = [
                make(&[&p, &s, &fa, &s, &q]),
                make(&[&p, &s, &fb, &s, &q]),
            ];
            assert_eq!(run(&files).len(), 3);
        }

        impl Prepared {
            fn clone_prepared(&self) -> Prepared {
                Prepared {
                    lines: self.lines,
                    rows: self.rows.clone(),
                }
            }
        }
    }
}
