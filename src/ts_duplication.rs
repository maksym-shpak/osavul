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
}
