//! Правило `empty-test`: спільні ідентифікатор і повідомлення та прив'язка мовних детекторів.
//! Детектор отримує вміст одного файла й повертає лише фізичні рядки; шлях, формат запису й
//! коди завершення належать рушію.

use crate::check::{Detector, Lang, Rule};
use crate::empty_rust_test;
use crate::ts_source::{line_of, line_starts, with_parsed};
use oxc_ast::ast::{Argument, CallExpression, Expression};
use oxc_ast_visit::{Visit, walk};

pub const EMPTY_TEST: Rule = Rule {
    id: "empty-test",
    message: "тест порожній: напишіть перевірку в тілі тесту або видаліть його",
    detectors: &[
        (Lang::Rust, rust as Detector),
        (Lang::TypeScript, typescript as Detector),
    ],
};

fn rust(source: &[u8], _name: &str) -> Result<Vec<usize>, String> {
    let text = std::str::from_utf8(source).map_err(|e| format!("вміст не є UTF-8: {e}"))?;
    empty_rust_test::find_empty_tests(text)
}

/// Будь-яка діагностика Oxc відхиляє файл (`with_parsed`); BOM знято до побудови карти рядків,
/// тож позиції лишаються фізичними.
fn typescript(source: &[u8], name: &str) -> Result<Vec<usize>, String> {
    with_parsed(name.as_bytes(), source, |text, program, _| {
        let mut finder = Finder {
            starts: line_starts(text),
            lines: Vec::new(),
        };
        finder.visit_program(program);
        finder.lines
    })
    .map_err(|e| e.reason().to_string())
}

struct Finder {
    starts: Vec<usize>,
    lines: Vec<usize>,
}

fn is_empty_function(arg: Option<&Argument<'_>>) -> bool {
    let Some(expr) = arg.and_then(Argument::as_expression) else {
        return false;
    };
    match expr.without_parentheses() {
        // Тіло-вираз у Oxc 0.160 — окремий варіант `ArrowFunctionBody`, а не блок.
        Expression::ArrowFunctionExpression(f) => f
            .body
            .as_function_body()
            .is_some_and(|b| b.statements.is_empty() && b.directives.is_empty()),
        Expression::FunctionExpression(f) => f
            .body
            .as_ref()
            .is_some_and(|b| b.statements.is_empty() && b.directives.is_empty()),
        _ => false,
    }
}

impl<'a> Visit<'a> for Finder {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Expression::Identifier(id) = call.callee.without_parentheses()
            && (id.name == "it" || id.name == "test")
            && call
                .arguments
                .first()
                .and_then(Argument::as_expression)
                .is_some_and(|e| {
                    matches!(
                        e.without_parentheses(),
                        Expression::StringLiteral(_) | Expression::TemplateLiteral(_)
                    )
                })
            && is_empty_function(call.arguments.get(1))
        {
            self.lines
                .push(line_of(&self.starts, id.span.start as usize) + 1);
        }
        walk::walk_call_expression(self, call);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(src: &str) -> Vec<usize> {
        typescript(src.as_bytes(), "a.ts").unwrap()
    }

    #[test]
    fn matches_direct_calls_with_all_four_conditions() {
        for src in [
            "it('a', () => {});",
            "test('a', () => {});",
            "it(`a`, () => {});",
            "it(`a${b}`, () => {});",
            "it('a', function () {});",
            "it('a', async () => {});",
            "it('a', async function named() {});",
            "it('a', (x, y: number) => { /* c */ // d\n });",
            "it('a', () => {}, 1000);",
            "(it)('a', () => {});",
            "it(('a'), () => {});",
            "it('a', (() => {}));",
            "it?.('a', () => {});",
            "it<T>('a', () => {});",
            "describe('d', () => { describe('e', () => { it('a', () => {}); }); });",
        ] {
            assert_eq!(lines(src), [1], "{src}");
        }
    }

    #[test]
    fn rejects_other_shapes() {
        for src in [
            "it.todo('a', () => {});",
            "it.skip('a', () => {});",
            "it.only('a', () => {});",
            "test.each([1])('a', () => {});",
            "a.it('a', () => {});",
            "xit('a', () => {});",
            "fit('a', () => {});",
            "it('a');",
            "it('a', 1);",
            "it('a', { timeout: 1 });",
            "it('a', () => undefined);",
            "it('a', () => ({}));",
            "it('a', () => { ; });",
            "it('a', () => { x(); });",
            "it('a', function () { return; });",
            "it('a', () => { 'use strict'; });",
            "it(tag`a`, () => {});",
            "it(a, () => {});",
            "it(1, () => {});",
            "it('a', (() => {}).bind(null));",
            "it();",
        ] {
            assert!(lines(src).is_empty(), "{src}");
        }
    }

    #[test]
    fn nested_and_repeated_calls_each_count() {
        assert_eq!(lines("it('a', () => {}); it('b', () => {});"), [1, 1]);
        assert_eq!(
            lines("it('a', () => {\n  it('b', () => {});\n  x();\n});"),
            [2]
        );
    }

    #[test]
    fn position_is_the_identifier_line_for_every_line_separator() {
        assert_eq!(lines("it(\n'a',\n() => {});"), [1]);
        assert_eq!(lines("(\n  it\n)('a', () => {});"), [2]);
        for sep in ["\n", "\r", "\r\n", "\u{2028}", "\u{2029}"] {
            assert_eq!(
                lines(&format!("x(){sep}it('a', () => {{}});")),
                [2],
                "{sep:?}"
            );
            assert_eq!(
                lines(&format!("{sep}{sep}test('a', () => {{}});")),
                [3],
                "{sep:?}"
            );
        }
    }

    #[test]
    fn bom_does_not_shift_lines_and_diagnostics_reject_the_file() {
        assert_eq!(lines("\u{feff}\nit('a', () => {});"), [2]);
        assert_eq!(
            typescript(b"it('a', () => {});\nlet x = ;", "a.ts"),
            Err("syntax-error".to_string())
        );
        assert_eq!(typescript(b"\xff", "a.ts"), Err("invalid-utf8".to_string()));
    }

    #[test]
    fn declaration_files_are_accepted() {
        assert!(typescript(b"declare function f(): void;\n", "a.d.ts").is_ok());
    }
}
