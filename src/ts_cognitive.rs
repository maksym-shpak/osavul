//! Методика `ts-cognitive` версії 1: синтаксичний підрахунок когнітивної складності
//! одного файла TypeScript. Зміна правил підрахунку потребує нової версії.
//!
//! Граничні випадки (код поза функціями, рівень `0`), якими зафіксовано правила:
//!
//! - `if`, `else if`, `else`, а в тілі `else if` ще один `if`: `1 + 1 + 1 + 2 = 5`.
//!   `else if` — фіксований `+1`, а його тіло лишається на рівні початкового `if` плюс один.
//! - Тернарник у гілці тернарника (також в альтернативній): `1 + 2 = 3`.
//! - У тілі `if` є `switch`, у його `case` є `if`: `1 + 1 + 3 = 5`; `switch` дає `+1`
//!   один раз, порожні `case` нічого не додають.
//! - `if` з умовою `a && b || c`: `1 + 2 = 3` (умова обходиться на рівні самого `if`).
//! - Зовнішній `if`, у тілі `try` з одним `if` і порожній `catch`: `1 + 2 + 2 = 5`;
//!   `try` рівня не підвищує, `catch` рахується з вкладеністю, а `if` у його тілі додав би `3`.
//! - Логічні послідовності: `a && b && c` = 1, `a && b || c` = 2,
//!   `a && b && c || d || e` = 2, `a && (b || c) && d` = 3.
//! - `async`, `await` і генератори власного приросту не дають.

use crate::ts_source::{ScoreError, with_program};
use oxc_ast::ast::{
    AccessorProperty, ArrowFunctionExpression, BreakStatement, CallExpression, CatchClause,
    ConditionalExpression, ContinueStatement, DoWhileStatement, Expression, ForInStatement,
    ForOfStatement, ForStatement, Function, IfStatement, LogicalExpression, LogicalOperator,
    MethodDefinition, MethodDefinitionKind, ObjectProperty, PropertyDefinition, PropertyKey,
    Statement, StaticBlock, SwitchStatement, TSInterfaceDeclaration, TSSignature, TSType,
    WhileStatement,
};
use oxc_ast_visit::{Visit, walk};

/// Бал файла за його шляхом (лише для вибору режиму `.d.ts`) і сирим вмістом.
pub fn score(path: &[u8], content: &[u8]) -> Result<u64, ScoreError> {
    with_program(path, content, |program| {
        let mut counter = Counter {
            total: 0,
            nesting: 0,
            in_function: false,
            name: None,
            class_method: false,
            pending_name: None,
        };
        counter.visit_program(program);
        counter.total
    })
}

struct Counter {
    total: u64,
    nesting: u64,
    /// Чи йде обхід у тілі функції (на відміну від коду поза функціями).
    in_function: bool,
    /// Власне ім'я найближчої функції для прямої рекурсії; стрілка його не має.
    name: Option<String>,
    /// Найближча функція — метод класу, тож `this.name()` теж є самовикликом.
    class_method: bool,
    /// Ім'я методу об'єктного літерала для найближчого функціонального виразу.
    pending_name: Option<String>,
}

impl Counter {
    // Вкладена функція піднімає рівень лише всередині іншої функції чи конструкції, що
    // вкладає; прямий ініціалізатор чи колбек на рівні модуля, а також метод класу
    // (`top`), починають з нуля.
    fn function<F: FnOnce(&mut Self)>(&mut self, top: bool, f: F) {
        let saved = (
            self.nesting,
            self.in_function,
            self.name.take(),
            self.class_method,
        );
        if top || !(self.in_function || self.nesting > 0) {
            self.nesting = 0;
        } else {
            self.nesting += 1;
        }
        self.in_function = true;
        // Ім'я для рекурсії активується лише на час тіла (див. `body`).
        self.name = None;
        self.class_method = false;
        f(self);
        (self.nesting, self.in_function, self.name, self.class_method) = saved;
    }

    // Ініціалізатор поля й статичний блок — код поза функцією з нульовим рівнем,
    // навіть у класі, оголошеному всередині функції.
    fn member_code<F: FnOnce(&mut Self)>(&mut self, f: F) {
        let saved = (
            self.nesting,
            self.in_function,
            self.name.take(),
            self.class_method,
        );
        self.nesting = 0;
        self.in_function = false;
        self.class_method = false;
        f(self);
        (self.nesting, self.in_function, self.name, self.class_method) = saved;
    }

    // Параметри обходяться без імені функції: самовиклик в ініціалізаторі параметра
    // не є самовикликом у тілі. Типи, повернення й `this`-параметр пропускаються.
    fn function_parts<'a>(&mut self, it: &Function<'a>, name: Option<String>, class_method: bool) {
        self.visit_formal_parameters(&it.params);
        self.name = name;
        self.class_method = class_method;
        if let Some(body) = &it.body {
            self.visit_function_body(body);
        }
    }

    /// `+1 + рівень` для конструкції, що вкладається.
    fn structural(&mut self) {
        self.total += 1 + self.nesting;
    }

    fn nested<F: FnOnce(&mut Self)>(&mut self, f: F) {
        self.nesting += 1;
        f(self);
        self.nesting -= 1;
    }

    // Гілки одного ланцюжка обробляються тут, а не через `visit_if_statement`, щоб
    // `else if` мав фіксований приріст і той самий рівень тіла, що й початковий `if`.
    fn if_chain<'a>(&mut self, it: &IfStatement<'a>, is_else_if: bool) {
        if is_else_if {
            self.total += 1;
        } else {
            self.structural();
        }
        self.visit_expression(&it.test);
        self.nested(|c| c.visit_statement(&it.consequent));
        match &it.alternate {
            Some(Statement::IfStatement(next)) => self.if_chain(next, true),
            Some(other) => {
                self.total += 1;
                self.nested(|c| c.visit_statement(other));
            }
            None => {}
        }
    }

    fn operand<'a>(&mut self, e: &Expression<'a>, last: &mut Option<LogicalOperator>) {
        match e.without_parentheses() {
            Expression::LogicalExpression(l) => self.chain(l, last),
            other => self.visit_expression(other),
        }
    }

    // Обхід у порядку запису: бал за початок і кожну зміну виду оператора.
    fn chain<'a>(&mut self, it: &LogicalExpression<'a>, last: &mut Option<LogicalOperator>) {
        self.operand(&it.left, last);
        if *last != Some(it.operator) {
            self.total += 1;
            *last = Some(it.operator);
        }
        self.operand(&it.right, last);
    }
}

impl<'a> Visit<'a> for Counter {
    fn visit_if_statement(&mut self, it: &IfStatement<'a>) {
        self.if_chain(it, false);
    }

    fn visit_conditional_expression(&mut self, it: &ConditionalExpression<'a>) {
        self.structural();
        self.visit_expression(&it.test);
        self.nested(|c| {
            c.visit_expression(&it.consequent);
            c.visit_expression(&it.alternate);
        });
    }

    fn visit_for_statement(&mut self, it: &ForStatement<'a>) {
        self.structural();
        if let Some(init) = &it.init {
            self.visit_for_statement_init(init);
        }
        if let Some(test) = &it.test {
            self.visit_expression(test);
        }
        if let Some(update) = &it.update {
            self.visit_expression(update);
        }
        self.nested(|c| c.visit_statement(&it.body));
    }

    fn visit_for_in_statement(&mut self, it: &ForInStatement<'a>) {
        self.structural();
        self.visit_for_statement_left(&it.left);
        self.visit_expression(&it.right);
        self.nested(|c| c.visit_statement(&it.body));
    }

    // Покриває і `for await…of`.
    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        self.structural();
        self.visit_for_statement_left(&it.left);
        self.visit_expression(&it.right);
        self.nested(|c| c.visit_statement(&it.body));
    }

    fn visit_while_statement(&mut self, it: &WhileStatement<'a>) {
        self.structural();
        self.visit_expression(&it.test);
        self.nested(|c| c.visit_statement(&it.body));
    }

    fn visit_do_while_statement(&mut self, it: &DoWhileStatement<'a>) {
        self.structural();
        self.nested(|c| c.visit_statement(&it.body));
        self.visit_expression(&it.test);
    }

    fn visit_catch_clause(&mut self, it: &CatchClause<'a>) {
        self.structural();
        if let Some(param) = &it.param {
            self.visit_catch_parameter(param);
        }
        self.nested(|c| c.visit_block_statement(&it.body));
    }

    fn visit_switch_statement(&mut self, it: &SwitchStatement<'a>) {
        self.total += 1;
        self.visit_expression(&it.discriminant);
        self.nested(|c| {
            for case in &it.cases {
                c.visit_switch_case(case);
            }
        });
    }

    fn visit_break_statement(&mut self, it: &BreakStatement<'a>) {
        if it.label.is_some() {
            self.total += 1;
        }
        walk::walk_break_statement(self, it);
    }

    fn visit_continue_statement(&mut self, it: &ContinueStatement<'a>) {
        if it.label.is_some() {
            self.total += 1;
        }
        walk::walk_continue_statement(self, it);
    }

    fn visit_logical_expression(&mut self, it: &LogicalExpression<'a>) {
        self.chain(it, &mut None);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        let _ = flags;
        let name = self
            .pending_name
            .take()
            .or_else(|| it.id.as_ref().map(|id| id.name.to_string()));
        self.function(false, |c| c.function_parts(it, name, false));
    }

    // Метод об'єктного літерала бере ім'я з простого необчислюваного ключа.
    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        if it.method
            && let PropertyKey::StaticIdentifier(id) = &it.key
        {
            self.pending_name = Some(id.name.to_string());
        }
        self.visit_property_key(&it.key);
        self.visit_expression(&it.value);
        self.pending_name = None;
    }

    // Конструкції рівня типів не виконуються: їхні ключі й вирази не рахуються.
    fn visit_ts_type(&mut self, _it: &TSType<'a>) {}

    fn visit_ts_signature(&mut self, _it: &TSSignature<'a>) {}

    fn visit_ts_interface_declaration(&mut self, _it: &TSInterfaceDeclaration<'a>) {}

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.pending_name = None;
        self.function(false, |c| walk::walk_arrow_function_expression(c, it));
    }

    fn visit_method_definition(&mut self, it: &MethodDefinition<'a>) {
        for decorator in &it.decorators {
            self.visit_decorator(decorator);
        }
        self.visit_property_key(&it.key);
        let name = match (&it.key, it.kind) {
            (PropertyKey::StaticIdentifier(id), MethodDefinitionKind::Method) => {
                Some(id.name.to_string())
            }
            _ => None,
        };
        let class_method = name.is_some();
        self.function(true, |c| c.function_parts(&it.value, name, class_method));
    }

    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        for decorator in &it.decorators {
            self.visit_decorator(decorator);
        }
        self.visit_property_key(&it.key);
        if let Some(value) = &it.value {
            self.member_code(|c| c.visit_expression(value));
        }
    }

    fn visit_accessor_property(&mut self, it: &AccessorProperty<'a>) {
        for decorator in &it.decorators {
            self.visit_decorator(decorator);
        }
        self.visit_property_key(&it.key);
        if let Some(value) = &it.value {
            self.member_code(|c| c.visit_expression(value));
        }
    }

    fn visit_static_block(&mut self, it: &StaticBlock<'a>) {
        self.member_code(|c| walk::walk_static_block(c, it));
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Some(name) = &self.name {
            let direct = match it.callee.without_parentheses() {
                Expression::Identifier(id) => id.name == name.as_str(),
                Expression::StaticMemberExpression(m) => {
                    self.class_method
                        && matches!(
                            m.object.without_parentheses(),
                            Expression::ThisExpression(_)
                        )
                        && m.property.name == name.as_str()
                }
                _ => false,
            };
            if direct {
                self.total += 1;
            }
        }
        walk::walk_call_expression(self, it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(src: &str) -> u64 {
        score(b"a.ts", src.as_bytes()).unwrap()
    }

    #[test]
    fn if_adds_one_plus_level_and_else_branches_add_one() {
        assert_eq!(s("if (a) {}"), 1);
        assert_eq!(s("if (a) {} else {}"), 2);
        assert_eq!(s("if (a) {} else if (b) {}"), 2);
        assert_eq!(s("if (a) {} else if (b) {} else {}"), 3);
    }

    #[test]
    fn else_if_chain_with_inner_if_in_else_if_body() {
        assert_eq!(
            s("if (a) {} else if (b) { if (c) {} } else {}"),
            1 + 1 + 2 + 1
        );
        assert_eq!(s("if (a) {} else if (b) { if (c) {} } else { x(); }"), 5);
    }

    #[test]
    fn nested_bodies_raise_level_for_if_and_else() {
        assert_eq!(s("if (a) { if (b) {} }"), 3);
        assert_eq!(s("if (a) {} else { if (b) {} }"), 1 + 1 + 2);
    }

    #[test]
    fn ternary_in_either_branch_of_ternary_costs_three() {
        assert_eq!(s("a ? b : c;"), 1);
        assert_eq!(s("a ? (b ? 1 : 2) : 3;"), 3);
        assert_eq!(s("a ? 1 : (b ? 2 : 3);"), 3);
    }

    #[test]
    fn ternary_test_stays_on_entry_level() {
        assert_eq!(s("(a ? b : c) ? 1 : 2;"), 2);
    }

    #[test]
    fn switch_counts_once_and_nests_its_body() {
        assert_eq!(s("switch (a) { case 1: case 2: default: }"), 1);
        assert_eq!(s("if (a) { switch (b) { case 1: case 2: if (c) {} } }"), 5);
        assert_eq!(s("if (a) { switch (b) { case 1: if (c) {} } }"), 5);
    }

    #[test]
    fn all_loop_forms_add_one_plus_level_and_nest_body() {
        for head in [
            "for (;;) {",
            "for (const k in o) {",
            "for (const v of o) {",
            "while (a) {",
        ] {
            assert_eq!(s(&format!("{head} }}")), 1, "{head}");
            assert_eq!(s(&format!("{head} if (b) {{}} }}")), 3, "{head}");
        }
        assert_eq!(s("do { if (b) {} } while (a);"), 3);
    }

    #[test]
    fn for_await_of_counts_like_for_of() {
        assert_eq!(s("async function f() { for await (const v of o) {} }"), 1);
    }

    #[test]
    fn loop_header_is_on_entry_level() {
        assert_eq!(
            s("if (a) { for (let i = x ? 1 : 2; i < n; i++) {} }"),
            1 + 2 + 2
        );
    }

    #[test]
    fn labeled_jumps_add_one_and_plain_jumps_zero() {
        assert_eq!(s("l: for (;;) { break l; }"), 2);
        assert_eq!(s("l: for (;;) { continue l; }"), 2);
        assert_eq!(s("for (;;) { break; }"), 1);
        assert_eq!(s("for (;;) { continue; }"), 1);
    }

    #[test]
    fn catch_nests_but_try_and_finally_do_not() {
        assert_eq!(s("try { if (a) {} } finally { if (b) {} }"), 2);
        assert_eq!(s("try {} catch (e) {}"), 1);
        assert_eq!(s("if (a) { try { if (b) {} } catch (e) {} }"), 1 + 2 + 2);
        assert_eq!(
            s("if (a) { try { if (b) {} } catch (e) { if (c) {} } }"),
            1 + 2 + 2 + 3
        );
    }

    #[test]
    fn level_is_restored_after_each_body() {
        assert_eq!(s("if (a) { if (b) {} } if (c) {}"), 3 + 1);
        assert_eq!(s("for (;;) { while (a) {} } while (b) {}"), 3 + 1);
    }

    #[test]
    fn logical_sequences_count_starts_and_kind_changes() {
        assert_eq!(s("a && b && c;"), 1);
        assert_eq!(s("a && b || c;"), 2);
        assert_eq!(s("a && b && c || d || e;"), 2);
        assert_eq!(s("a ?? b ?? c;"), 1);
        assert_eq!(s("(a ?? b) || c;"), 2);
        assert_eq!(s("a;"), 0);
    }

    #[test]
    fn parentheses_are_transparent_in_logical_sequence() {
        assert_eq!(s("a && (b || c) && d;"), 3);
        assert_eq!(s("(a && b) && c;"), 1);
    }

    #[test]
    fn if_condition_adds_logical_sequences_to_if() {
        assert_eq!(s("if (a && b || c) {}"), 3);
    }

    #[test]
    fn call_separates_logical_chains() {
        assert_eq!(s("a && f(b && c) && d;"), 2);
    }

    #[test]
    fn negation_separates_logical_chains() {
        assert_eq!(s("a && !(b && c) && d;"), 2);
    }

    #[test]
    fn type_assertion_separates_logical_chains() {
        assert_eq!(s("a && ((b && c) as boolean) && d;"), 2);
    }

    #[test]
    fn ternary_separates_logical_chains() {
        assert_eq!(s("a && (b ? c && d : e) && f;"), 1 + 1 + 1);
    }

    #[test]
    fn zero_contribution_constructs_add_nothing() {
        assert_eq!(s("try {} finally {}"), 0);
        assert_eq!(s("a?.b?.(c);"), 0);
        assert_eq!(s("a ??= b; a ||= b; a &&= b;"), 0);
        assert_eq!(s("function f(x = 1) { return x; }"), 0);
        assert_eq!(s("function g() { throw new Error(); }"), 0);
        assert_eq!(
            s("type T<A> = A extends string ? 1 : 2; interface I { m(): void }"),
            0
        );
        assert_eq!(
            s("interface I { [f(a || b)](): void } let x: { [f(a || b)]: 1 };"),
            0
        );
        assert_eq!(s("declare function d(a: number): void;"), 0);
        assert_eq!(s("function o(a: number): void; function o(a: any) {}"), 0);
    }

    #[test]
    fn async_await_and_generators_do_not_change_score() {
        let body = "if (a) { if (b) {} }";
        let plain = s(&format!("function f() {{ {body} }}"));
        assert_eq!(plain, 3);
        assert_eq!(
            s(&format!("async function f() {{ await x; {body} }}")),
            plain
        );
        assert_eq!(s(&format!("function* f() {{ yield 1; {body} }}")), plain);
    }

    #[test]
    fn executable_subexpressions_of_zero_constructs_are_counted() {
        assert_eq!(s("a?.[b ? 1 : 2];"), 1);
        assert_eq!(s("a ||= b && c;"), 1);
        assert_eq!(s("function f(x = a ? 1 : 2) {}"), 1);
        assert_eq!(s("throw a ? b : c;"), 1);
        assert_eq!(s("function f() { return a || b; }"), 1);
    }

    #[test]
    fn top_level_and_class_members_start_at_level_zero() {
        assert_eq!(s("namespace N { if (a) {} }"), 1);
        assert_eq!(s("class C { x = a ? 1 : 2; static { if (a) {} } }"), 2);
    }

    #[test]
    fn arrow_in_callback_inside_function_is_nested() {
        assert_eq!(s("function f() { arr.map(x => { if (x) {} }); }"), 2);
    }

    #[test]
    fn module_level_arrow_and_callback_start_at_zero() {
        assert_eq!(s("const f = () => { if (a) {} };"), 1);
        assert_eq!(s("arr.map(x => { if (x) {} });"), 1);
        assert_eq!(s("const g = function () { if (a) {} };"), 1);
    }

    #[test]
    fn level_is_restored_after_function() {
        assert_eq!(
            s("function f() { g(() => { if (a) {} }); if (b) {} }"),
            2 + 1
        );
        assert_eq!(s("if (a) { const h = () => {}; } if (b) {}"), 2);
    }

    #[test]
    fn function_in_nesting_body_raises_level() {
        assert_eq!(s("if (a) { const h = () => { if (b) {} }; }"), 1 + 3);
    }

    #[test]
    fn object_literal_methods_and_accessors_are_nested_functions() {
        assert_eq!(
            s("function f() { return { m() { if (a) {} }, get g() { if (b) {} } }; }"),
            2 + 2
        );
        assert_eq!(s("const o = { m() { if (a) {} } };"), 1);
    }

    #[test]
    fn class_method_starts_at_zero_even_in_local_class() {
        assert_eq!(s("class C { m() { if (a) {} } }"), 1);
        assert_eq!(
            s("function f() { if (a) { class C { m() { if (b) {} } } } }"),
            1 + 1
        );
    }

    #[test]
    fn class_field_and_static_block_start_at_zero_even_in_local_class() {
        assert_eq!(
            s("function f() { if (a) { class C { x = b ? 1 : 2; static { if (c) {} } } } }"),
            1 + 1 + 1
        );
    }

    #[test]
    fn callback_in_class_method_inherits_function_nesting() {
        assert_eq!(s("class C { m() { arr.map(x => { if (x) {} }); } }"), 2);
    }

    #[test]
    fn parameter_default_ternary_counts_in_function_context() {
        assert_eq!(s("function f(x = a ? 1 : 2) {}"), 1);
        assert_eq!(s("function f() { return (x = a ? 1 : 2) => x; }"), 2);
    }

    #[test]
    fn direct_self_calls_count_per_call() {
        assert_eq!(s("function f() { f(); f(); }"), 2);
        assert_eq!(s("function f(n) { return n && f(n - 1); }"), 2);
        assert_eq!(s("function r(v = r(0)) { return v; }"), 0);
        assert_eq!(s("function r(v = a ? r(0) : 1) { return r(v); }"), 2);
        assert_eq!(
            s("const o = { down(n) { return n > 0 ? down(n - 1) : 0; } };"),
            2
        );
    }

    #[test]
    fn call_of_outer_function_from_nested_arrow_is_not_recursion() {
        assert_eq!(s("function f() { arr.map(() => f()); }"), 0);
    }

    #[test]
    fn this_call_is_recursion_only_for_class_method_with_same_name() {
        assert_eq!(s("class C { m() { this.m(); m(); this.n(); } }"), 2);
        assert_eq!(s("function m() { this.m(); }"), 0);
    }

    #[test]
    fn indirect_recursion_is_outside_v1() {
        assert_eq!(s("function a() { b(); } function b() { a(); }"), 0);
        assert_eq!(s("const f = () => { f(); };"), 0);
        assert_eq!(s("function f() { const g = f; g(); }"), 0);
        assert_eq!(s("function f() { o.f(); }"), 0);
    }

    #[test]
    fn async_and_generator_forms_keep_same_score() {
        let body = "if (a) { for (const x of y) { if (x) {} } }";
        let plain = s(&format!("function f() {{ {body} }}"));
        assert_eq!(plain, 1 + 2 + 3);
        assert_eq!(s(&format!("async function f() {{ {body} }}")), plain);
        assert_eq!(s(&format!("function* f() {{ {body} }}")), plain);
        assert_eq!(s(&format!("const f = async () => {{ {body} }};")), plain);
    }

    #[test]
    fn whitepaper_sum_of_primes_is_seven() {
        let src = "function sumOfPrimes(max: number): number {
            let total = 0;
            OUT: for (let i = 1; i <= max; ++i) {
                for (let j = 2; j < i; ++j) {
                    if (i % j === 0) {
                        continue OUT;
                    }
                }
                total += i;
            }
            return total;
        }";
        assert_eq!(s(src), 1 + 2 + 3 + 1);
    }

    #[test]
    fn whitepaper_get_words_is_one() {
        let src = "function getWords(number: number): string {
            switch (number) {
                case 1: return 'one';
                case 2: return 'a couple';
                case 3: return 'a few';
                default: return 'lots';
            }
        }";
        assert_eq!(s(src), 1);
    }

    #[test]
    fn whitepaper_overridden_symbol_from_is_nineteen() {
        let src =
            "function overriddenSymbolFrom(classType: Type, name: string): Symbol | undefined {
            if (classType.unknown) {
                return undefined;
            }
            for (const candidate of classType.candidates) {
                if (candidate.name === name && candidate.isMember) {
                    if (candidate.canOverride) {
                        const result = check(candidate);
                        if (result.unknown) {
                            return undefined;
                        } else if (result.positive) {
                            if (flag) {
                                return candidate;
                            }
                        }
                    }
                }
            }
            if (flag) {
                return undefined;
            }
            return undefined;
        }";
        assert_eq!(s(src), 1 + 1 + (2 + 1) + 3 + 4 + 5 + 1 + 1);
    }

    #[test]
    fn declaration_file_is_accepted() {
        assert_eq!(
            score(b"types.d.ts", b"declare function f(a: number): void;\n"),
            Ok(0)
        );
    }

    #[test]
    fn leading_bom_is_stripped() {
        assert_eq!(score(b"a.ts", b"\xef\xbb\xbfif (a) {}"), Ok(1));
    }

    #[test]
    fn invalid_utf8_is_reported() {
        assert_eq!(score(b"a.ts", b"\xff"), Err(ScoreError::InvalidUtf8));
    }

    #[test]
    fn syntax_error_is_reported() {
        assert_eq!(score(b"a.ts", b"if ("), Err(ScoreError::Syntax));
    }

    #[test]
    fn recovered_tree_with_diagnostic_is_error() {
        assert_eq!(
            score(b"a.ts", b"let x = 1; let x = 2; if (a) {} const;"),
            Err(ScoreError::Syntax)
        );
    }

    #[test]
    fn corpus_matches_expected_tsv() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts-cognitive");
        let table = std::fs::read_to_string(dir.join("expected.tsv")).unwrap();
        let mut rows = 0;
        for line in table.lines().skip(1) {
            let cols: Vec<&str> = line.split('\t').collect();
            let content = std::fs::read(dir.join(cols[0])).unwrap();
            let expected: u64 = cols[2].parse().unwrap();
            assert_eq!(
                score(cols[0].as_bytes(), &content),
                Ok(expected),
                "{}",
                cols[0]
            );
            rows += 1;
        }
        assert_eq!(rows, 4);
    }
}
