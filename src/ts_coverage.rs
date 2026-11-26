//! Межі рядків і підрахунок для методики `ts-coverage`: один розбір основного файла дає число
//! фізичних рядків (для перевірки номерів LCOV) і, для файла без `SF`, кількість різних рядків
//! початку вузлів ESTree, які SonarJS вважає виконуваними.

use crate::ts_source::{ScoreError, line_of, line_starts, with_parsed};
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use std::collections::BTreeSet;

/// Результат розбору файла.
#[derive(Debug, PartialEq, Eq)]
pub struct Lines {
    /// Кількість фізичних рядків; допустимі номери LCOV — `1..=physical`.
    pub physical: u64,
    /// Різні рядки початку погоджених вузлів (для файла без `SF`).
    pub statement_lines: u64,
}

/// Перевіряє файл спільним парсером. Шлях лише обирає режим `.d.ts`.
pub fn analyze(path: &[u8], content: &[u8]) -> Result<Lines, ScoreError> {
    with_parsed(path, content, |text, program, _| {
        let starts = line_starts(text);
        let mut walker = Starts {
            starts: &starts,
            lines: BTreeSet::new(),
        };
        walker.visit_program(program);
        Lines {
            physical: starts.len() as u64,
            statement_lines: walker.lines.len() as u64,
        }
    })
}

struct Starts<'s> {
    starts: &'s [usize],
    lines: BTreeSet<usize>,
}

impl Starts<'_> {
    fn mark(&mut self, span: oxc_span::Span) {
        self.lines.insert(line_of(self.starts, span.start as usize));
    }
}

macro_rules! counted {
    ($($method:ident, $walk:ident, $ty:ident;)*) => {
        impl<'a> Visit<'a> for Starts<'_> {
            $(
                fn $method(&mut self, it: &$ty<'a>) {
                    self.mark(it.span);
                    walk::$walk(self, it);
                }
            )*
            fn visit_debugger_statement(&mut self, it: &DebuggerStatement) {
                self.mark(it.span);
                walk::walk_debugger_statement(self, it);
            }
            // ESTree подає директиву як `ExpressionStatement`; в Oxc це окремий вузол.
            fn visit_directive(&mut self, it: &Directive<'a>) {
                self.mark(it.span);
                walk::walk_directive(self, it);
            }
        }
    };
}

counted! {
    visit_expression_statement, walk_expression_statement, ExpressionStatement;
    visit_if_statement, walk_if_statement, IfStatement;
    visit_labeled_statement, walk_labeled_statement, LabeledStatement;
    visit_break_statement, walk_break_statement, BreakStatement;
    visit_continue_statement, walk_continue_statement, ContinueStatement;
    visit_with_statement, walk_with_statement, WithStatement;
    visit_switch_statement, walk_switch_statement, SwitchStatement;
    visit_return_statement, walk_return_statement, ReturnStatement;
    visit_throw_statement, walk_throw_statement, ThrowStatement;
    visit_try_statement, walk_try_statement, TryStatement;
    visit_while_statement, walk_while_statement, WhileStatement;
    visit_do_while_statement, walk_do_while_statement, DoWhileStatement;
    visit_for_statement, walk_for_statement, ForStatement;
    visit_for_in_statement, walk_for_in_statement, ForInStatement;
    visit_for_of_statement, walk_for_of_statement, ForOfStatement;
    visit_variable_declaration, walk_variable_declaration, VariableDeclaration;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn count(src: &str) -> u64 {
        analyze(b"a.ts", src.as_bytes()).unwrap().statement_lines
    }

    fn physical(src: &str) -> u64 {
        analyze(b"a.ts", src.as_bytes()).unwrap().physical
    }

    #[test]
    fn every_listed_kind_counts_on_its_own_line() {
        let cases = [
            "x();",
            "if (a) {}",
            "l: for (;;) { break l; }",
            "while (1) { break; }",
            "while (1) { continue; }",
            "do {} while (a);",
            "for (const k in o) {}",
            "for (const v of o) {}",
            "switch (a) {}",
            "function f() { return 1; }",
            "throw 1;",
            "try {} catch {}",
            "debugger;",
            "var v = 1;",
        ];
        for src in cases {
            assert!(count(src) >= 1, "{src}");
        }
        // Кожен вид на окремому рядку: перевіряємо точну кількість для кількох.
        assert_eq!(count("x();\nif (a) {}\ndebugger;\nthrow 1;\nvar v;"), 5);
        assert_eq!(count("switch (a) {\n}\ntry {\n} finally {\n}"), 2);
        assert_eq!(count("a: {\n  b: {\n  }\n}"), 2);
    }

    #[test]
    fn with_statement_counts() {
        // `with` заборонений у strict-режимі модулів, тож перевіряємо скрипт без import/export.
        assert_eq!(count("with (o) {\n  x;\n}"), 2);
    }

    #[test]
    fn directives_count_in_program_and_function() {
        assert_eq!(count("'use strict';\nfunction f() {\n  'use asm';\n}"), 2);
    }

    #[test]
    fn loop_header_declarations_and_nested_nodes() {
        assert_eq!(count("for (let i = 0; i < 1; i++) {}"), 1);
        assert_eq!(count("for (let i = 0;\n  i < 1;\n  i++)\n  x();"), 2);
        assert_eq!(count("for (const v of o) {\n  const w = v;\n}"), 2);
    }

    #[test]
    fn multiline_node_counts_only_at_start() {
        assert_eq!(count("return_(\n  1,\n  2\n);"), 1);
        assert_eq!(count("const a = {\n  b: 1,\n  c: [\n    2,\n  ],\n};"), 1);
    }

    #[test]
    fn several_starts_on_one_line_count_once() {
        assert_eq!(count("let a = 1; let b = 2; if (a) { b = 3; }"), 1);
    }

    #[test]
    fn other_nodes_contribute_nothing_alone() {
        assert_eq!(count("type T = number;\ninterface I { x: number }\n"), 0);
        assert_eq!(count("function f() {}\nclass C { x = 1; m() {} }\n"), 0);
        assert_eq!(count("import a from 'a';\nexport { a };\n;\n{}\n"), 0);
        assert_eq!(count("enum E { A }\nnamespace N {}\n"), 0);
    }

    #[test]
    fn declaration_files_use_declaration_mode() {
        let src = b"declare const x: number;\ndeclare function f(): void;\n";
        let lines = analyze(b"a.d.ts", src).unwrap();
        assert_eq!(lines.statement_lines, 1);
        // Ті самі байти як звичайний .ts — теж коректні; режим не змінює межу.
        assert_eq!(lines.physical, 3);
    }

    #[test]
    fn physical_lines_follow_ecmascript_terminators() {
        assert_eq!(physical(""), 1);
        assert_eq!(physical("x;"), 1);
        assert_eq!(physical("x;\n"), 2);
        assert_eq!(physical("x;\r\ny;"), 2);
        assert_eq!(physical("x;\ry;"), 2);
        assert_eq!(physical("x;\u{2028}y;"), 2);
        assert_eq!(physical("x;\u{2029}y;"), 2);
        assert_eq!(physical("x;\n\r\n\r"), 4);
    }

    #[test]
    fn bom_and_multibyte_text_keep_line_positions() {
        let src = "\u{feff}'ü';\nлітера();\n\u{2028}z();";
        let lines = analyze(b"a.ts", src.as_bytes()).unwrap();
        assert_eq!(lines.physical, 4);
        assert_eq!(lines.statement_lines, 3);
        assert_eq!(count("\"日本\"; x();\n\"ü\"; y();"), 2);
    }

    #[test]
    fn invalid_utf8_and_syntax_are_rejected() {
        assert_eq!(analyze(b"a.ts", b"x;\xff"), Err(ScoreError::InvalidUtf8));
        assert_eq!(analyze(b"a.ts", b"const = ;"), Err(ScoreError::Syntax));
    }

    #[test]
    fn corpus_source_files_match_expected_tsv() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts-coverage");
        let table = std::fs::read_to_string(dir.join("expected.tsv")).unwrap();
        let mut rows = 0;
        for row in table.lines().skip(1) {
            let f: Vec<&str> = row.split('\t').collect();
            if f[2].contains('#') || f[2].starts_with('@') {
                continue;
            }
            let content = std::fs::read(dir.join(f[2])).unwrap();
            match (f[5], analyze(f[2].as_bytes(), &content)) {
                ("-", Ok(l)) => {
                    assert_eq!(l.physical, f[3].parse::<u64>().unwrap(), "{}", f[0]);
                    assert_eq!(l.statement_lines, f[6].parse::<u64>().unwrap(), "{}", f[0]);
                    assert_eq!(l.statement_lines, f[7].parse::<u64>().unwrap(), "{}", f[0]);
                    assert_eq!((f[8], f[9]), ("0", "0"), "{}", f[0]);
                }
                ("syntax-error", Err(ScoreError::Syntax)) => {}
                (want, got) => panic!("{}: очікувано {want}, отримано {got:?}", f[0]),
            }
            rows += 1;
        }
        assert_eq!(rows, 2);
    }
}
