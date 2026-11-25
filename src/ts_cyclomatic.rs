//! Методика `ts-cyclomatic` версії 1: синтаксичний підрахунок для одного файла TypeScript.
//! Зміна правил підрахунку потребує нової версії.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ArrowFunctionExpression, ConditionalExpression, DoWhileStatement, ForInStatement,
    ForOfStatement, ForStatement, Function, IfStatement, LogicalExpression, SwitchCase,
    WhileStatement,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;

#[derive(Debug, PartialEq, Eq)]
pub enum ScoreError {
    InvalidUtf8,
    Syntax,
}

impl ScoreError {
    /// Стабільна причина для звіту.
    pub fn reason(&self) -> &'static str {
        match self {
            ScoreError::InvalidUtf8 => "invalid-utf8",
            ScoreError::Syntax => "syntax-error",
        }
    }
}

/// Бал файла за його шляхом (лише для вибору режиму `.d.ts`) і сирим вмістом.
pub fn score(path: &[u8], content: &[u8]) -> Result<u64, ScoreError> {
    let text = std::str::from_utf8(content).map_err(|_| ScoreError::InvalidUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let source_type = if path.ends_with(b".d.ts") {
        SourceType::d_ts()
    } else {
        SourceType::ts()
    };
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, text, source_type).parse();
    // Відновлене дерево з діагностикою теж є помилкою.
    if !ret.diagnostics.is_empty() {
        return Err(ScoreError::Syntax);
    }
    let mut counter = Counter(0);
    counter.visit_program(&ret.program);
    Ok(counter.0)
}

struct Counter(u64);

impl<'a> Visit<'a> for Counter {
    // Метод, конструктор і аксесор — це `Function` у `MethodDefinition`, тому рахуються тут
    // один раз; сигнатури й абстрактні методи не мають тіла.
    fn visit_function(&mut self, it: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        if it.body.is_some() {
            self.0 += 1;
        }
        walk::walk_function(self, it, flags);
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.0 += 1;
        walk::walk_arrow_function_expression(self, it);
    }

    fn visit_if_statement(&mut self, it: &IfStatement<'a>) {
        self.0 += 1;
        walk::walk_if_statement(self, it);
    }

    fn visit_conditional_expression(&mut self, it: &ConditionalExpression<'a>) {
        self.0 += 1;
        walk::walk_conditional_expression(self, it);
    }

    fn visit_for_statement(&mut self, it: &ForStatement<'a>) {
        self.0 += 1;
        walk::walk_for_statement(self, it);
    }

    fn visit_for_in_statement(&mut self, it: &ForInStatement<'a>) {
        self.0 += 1;
        walk::walk_for_in_statement(self, it);
    }

    // Покриває і `for await…of`.
    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        self.0 += 1;
        walk::walk_for_of_statement(self, it);
    }

    fn visit_while_statement(&mut self, it: &WhileStatement<'a>) {
        self.0 += 1;
        walk::walk_while_statement(self, it);
    }

    fn visit_do_while_statement(&mut self, it: &DoWhileStatement<'a>) {
        self.0 += 1;
        walk::walk_do_while_statement(self, it);
    }

    // `case` і `default` — один вузол `SwitchCase`.
    fn visit_switch_case(&mut self, it: &SwitchCase<'a>) {
        self.0 += 1;
        walk::walk_switch_case(self, it);
    }

    // `||`, `&&`, `??`: усі три оператори лише в `LogicalExpression`.
    fn visit_logical_expression(&mut self, it: &LogicalExpression<'a>) {
        self.0 += 1;
        walk::walk_logical_expression(self, it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(src: &str) -> u64 {
        score(b"a.ts", src.as_bytes()).unwrap()
    }

    #[test]
    fn acceptance_examples() {
        assert_eq!(ts("function f() { return 1; }"), 1);
        assert_eq!(ts("function f(a) { if (a) { return 1; } else { return 2; } }"), 2);
        assert_eq!(ts("function f(a, b) { if (a && b) { return 1; } return 2; }"), 3);
        assert_eq!(score(b"a.d.ts", b"declare function f(): void;"), Ok(0));
        assert_eq!(ts("const x = a ? 1 : 2;"), 1);
        assert_eq!(ts("switch (a) { case 1: break; default: break; }"), 2);
    }

    #[test]
    fn zero_is_a_valid_score() {
        assert_eq!(ts(""), 0);
        assert_eq!(ts("const x = 1;"), 0);
    }

    #[test]
    fn function_forms_count_once_each() {
        assert_eq!(ts("function f() { function g() {} }"), 2);
        assert_eq!(ts("const f = function () {};"), 1);
        assert_eq!(ts("const f = () => 1; const g = () => { return 1; };"), 2);
        assert_eq!(
            ts("class A { constructor() {} m() {} get x() { return 1; } set x(v) {} static s() {} }"),
            5
        );
        assert_eq!(ts("const o = { m() {}, get x() { return 1; } };"), 2);
    }

    #[test]
    fn signatures_without_implementation_score_zero() {
        assert_eq!(ts("function f(): void; function f(a?: number) {}"), 1);
        assert_eq!(ts("abstract class A { abstract m(): void; n(): void; }"), 0);
        assert_eq!(ts("class A { m(): void; m(a?: number) {} }"), 1);
        assert_eq!(ts("declare function f(): void; interface I { m(): void; }"), 0);
        assert_eq!(ts("declare class A { m(): void; }"), 0);
    }

    #[test]
    fn each_increment_adds_one() {
        assert_eq!(ts("if (a) {} else if (b) {}"), 2);
        assert_eq!(ts("for (;;) {}"), 1);
        assert_eq!(ts("for (const k in o) {}"), 1);
        assert_eq!(ts("for (const v of o) {}"), 1);
        assert_eq!(ts("async function f() { for await (const v of o) {} }"), 2);
        assert_eq!(ts("while (a) {}"), 1);
        assert_eq!(ts("do {} while (a);"), 1);
        assert_eq!(ts("const x = (a || b) ?? (c && d);"), 3);
        assert_eq!(ts("switch (a) { case 1: case 2: default: }"), 3);
    }

    #[test]
    fn non_incrementing_constructs_score_zero() {
        assert_eq!(ts("switch (a) {}"), 0);
        assert_eq!(ts("try { a(); } catch (e) { b(); } finally { c(); }"), 0);
        assert_eq!(ts("a?.b?.(); a ||= b; a &&= b; a ??= b;"), 0);
        assert_eq!(ts("if (a) {} else {}"), 1);
    }

    #[test]
    fn type_level_syntax_is_not_executable() {
        assert_eq!(ts("type T<A> = A extends string ? 1 : 2;"), 0);
        assert_eq!(ts("type U = (a: number) => void; interface I { f: () => void }"), 0);
        assert_eq!(ts("function f(a = b ? 1 : 2) {}"), 2);
    }

    #[test]
    fn code_outside_functions_has_no_base_score() {
        assert_eq!(ts("if (a && b) {}"), 2);
        assert_eq!(ts("namespace N { if (a) {} const x = b ?? c; }"), 2);
        assert_eq!(ts("class A { x = a ? 1 : 2; static { if (a) {} } }"), 2);
        assert_eq!(ts("class A { static x = () => 1; }"), 1);
    }

    #[test]
    fn nested_executable_expressions_are_counted() {
        assert_eq!(ts("function f(a) { return a ? (b ? 1 : 2) : g(c || d); }"), 4);
    }

    #[test]
    fn bom_does_not_change_score() {
        let plain = "function f(a) { if (a) {} }";
        let bom = format!("\u{feff}{plain}");
        assert_eq!(score(b"a.ts", bom.as_bytes()), Ok(2));
        assert_eq!(ts(plain), 2);
    }

    #[test]
    fn failures_have_distinct_reasons() {
        let invalid = score(b"a.ts", b"const x = '\xff';").unwrap_err();
        assert_eq!(invalid, ScoreError::InvalidUtf8);
        let syntax = score(b"a.ts", b"function (").unwrap_err();
        assert_eq!(syntax, ScoreError::Syntax);
        assert_ne!(invalid.reason(), syntax.reason());
    }

    #[test]
    fn recovered_tree_with_diagnostics_is_an_error() {
        // Парсер відновлюється після цієї помилки й будує дерево.
        assert_eq!(score(b"a.ts", b"let x = 1; let x = 2; if (a) {} const;"), Err(ScoreError::Syntax));
        assert_eq!(score(b"a.ts", b"class A { constructor() {} constructor() {} } function f( {"), Err(ScoreError::Syntax));
    }

    #[test]
    fn corpus_matches_expected_tsv() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts-cyclomatic");
        let table = std::fs::read_to_string(dir.join("expected.tsv")).unwrap();
        let mut rows = 0;
        for line in table.lines().skip(1) {
            let cols: Vec<&str> = line.split('\t').collect();
            let content = std::fs::read(dir.join(cols[0])).unwrap();
            let expected: u64 = cols[2].parse().unwrap();
            assert_eq!(score(cols[0].as_bytes(), &content), Ok(expected), "{}", cols[0]);
            rows += 1;
        }
        assert_eq!(rows, 2);
    }
}
