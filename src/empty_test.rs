//! Правило `empty-test`: спільні ідентифікатор і повідомлення та прив'язка мовних детекторів.
//! Детектор отримує вміст одного файла й повертає лише фізичні рядки; шлях, формат запису й
//! коди завершення належать рушію.

use crate::check::{Detector, Lang, Rule};
use crate::empty_rust_test;

pub const EMPTY_TEST: Rule = Rule {
    id: "empty-test",
    message: "тест порожній: напишіть перевірку в тілі тесту або видаліть його",
    detectors: &[(Lang::Rust, rust as Detector)],
};

fn rust(source: &[u8], _name: &str) -> Result<Vec<usize>, String> {
    let text = std::str::from_utf8(source).map_err(|e| format!("вміст не є UTF-8: {e}"))?;
    empty_rust_test::find_empty_tests(text)
}
