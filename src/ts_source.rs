//! Спільний розбір TypeScript для всіх методик (дерево або токени): декодування, BOM, режим `.d.ts`, діагностики.

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_parser::config::TokensParserConfig;
use oxc_parser::{ParseOptions, Parser, Token};
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

fn prepare<'a>(path: &[u8], content: &'a [u8]) -> Result<(&'a str, SourceType), ScoreError> {
    let text = std::str::from_utf8(content).map_err(|_| ScoreError::InvalidUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let source_type = if path.ends_with(b".d.ts") {
        SourceType::d_ts()
    } else {
        SourceType::ts()
    };
    Ok((text, source_type))
}

fn options() -> ParseOptions {
    ParseOptions {
        parse_regular_expression: true,
        ..ParseOptions::default()
    }
}

/// Розбирає файл (шлях лише обирає режим `.d.ts`) і виконує `walk` над деревом,
/// поки живе алокатор.
pub fn with_program<F>(path: &[u8], content: &[u8], walk: F) -> Result<u64, ScoreError>
where
    F: FnOnce(&Program<'_>) -> u64,
{
    let (text, source_type) = prepare(path, content)?;
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, text, source_type)
        .with_options(options())
        .parse();
    // Відновлене дерево з діагностикою теж є помилкою.
    if !ret.diagnostics.is_empty() {
        return Err(ScoreError::Syntax);
    }
    Ok(walk(&ret.program))
}

/// Те саме, але замість дерева віддає текст без BOM і токени коду в порядку джерела;
/// їхні байтові межі відраховано саме від цього тексту.
pub fn with_tokens<F>(path: &[u8], content: &[u8], walk: F) -> Result<u64, ScoreError>
where
    F: FnOnce(&str, &[Token]) -> u64,
{
    let (text, source_type) = prepare(path, content)?;
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, text, source_type)
        .with_options(options())
        .with_config(TokensParserConfig)
        .parse();
    if !ret.diagnostics.is_empty() {
        return Err(ScoreError::Syntax);
    }
    Ok(walk(text, ret.tokens.as_slice()))
}
