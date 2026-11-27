//! Rust-реалізація правила `empty-test`: синтаксис перевіряє локальний `rustc`, а межі
//! функцій, атрибутів і тіл шукає вузький обхід токенів того самого тексту.

use std::io::Write;
use std::process::{Command, Stdio};

/// Повертає фізичні номери рядків `fn` порожніх тестів у порядку появи.
///
/// `Err`, якщо `rustc` не запустився, текст не розбирається або дужки незбалансовані.
pub fn find_empty_tests(source: &str) -> Result<Vec<usize>, String> {
    check_syntax(source)?;
    let tokens = lex(source.strip_prefix('\u{feff}').unwrap_or(source))?;
    let close_of = match_groups(&tokens)?;
    let mut lines = Vec::new();
    walk_items(&tokens, &close_of, 0, tokens.len(), &mut lines);
    Ok(lines)
}

/// Перевіряє синтаксис через локальний `rustc`; `Err` — `rustc` не запустився або відхилив текст.
fn check_syntax(source: &str) -> Result<(), String> {
    // -Z потребує RUSTC_BOOTSTRAP; змінна ставиться лише дочірньому процесу.
    let mut child = Command::new("rustc")
        .args(["--edition", "2024", "-Z", "parse-crate-root-only", "-"])
        .env("RUSTC_BOOTSTRAP", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("не вдалося запустити локальний rustc: {e}"))?;
    let mut stdin = child.stdin.take().expect("stdin запитано");
    let data = source.as_bytes().to_vec();
    // Окремий потік, щоб великий вхід не заблокував обмін через канали.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&data);
    });
    let out = child
        .wait_with_output()
        .map_err(|e| format!("rustc не завершився коректно: {e}"))?;
    let _ = writer.join();
    if out.status.success() {
        Ok(())
    } else {
        let diag = String::from_utf8_lossy(&out.stderr);
        Err(format!(
            "розбір не вдався (потрібен rustc 1.100.0 із режимом -Z parse-crate-root-only):\n{}",
            diag.trim_end()
        ))
    }
}

#[derive(Debug, PartialEq)]
enum Kind {
    Ident(String),
    Punct(char),
    Literal,
    Open(char),
    Close,
    DocOuter,
    DocInner,
}

struct Token {
    kind: Kind,
    line: usize,
}

/// Пробіл або знак напрямку `\u{200e}`/`\u{200f}`, який не може входити до ідентифікатора.
fn is_pattern_ws(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{200e}' | '\u{200f}')
}

/// Передумова: текст уже пройшов розбір `rustc`, тому поза літералами й коментарями будь-який
/// не-ASCII символ, що не є пробілом, належить ідентифікатору. Тож таблиці XID Unicode не потрібні.
fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic() || (!c.is_ascii() && !is_pattern_ws(c))
}

/// Діє та сама передумова, що й для `is_ident_start`.
fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric() || (!c.is_ascii() && !is_pattern_ws(c))
}

/// Після `#!` пропускає пробіли та звичайні коментарі (через рядки) й повертає позицію
/// наступного значущого символа.
fn skip_trivia(c: &[char], mut j: usize) -> usize {
    loop {
        match (c.get(j), c.get(j + 1)) {
            (Some(&x), _) if is_pattern_ws(x) => j += 1,
            (Some('/'), Some('/')) => {
                while j < c.len() && c[j] != '\n' {
                    j += 1;
                }
            }
            (Some('/'), Some('*')) => {
                let mut depth = 0;
                while j < c.len() {
                    if c[j] == '/' && c.get(j + 1) == Some(&'*') {
                        depth += 1;
                        j += 2;
                    } else if c[j] == '*' && c.get(j + 1) == Some(&'/') {
                        depth -= 1;
                        j += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        j += 1;
                    }
                }
            }
            _ => return j,
        }
    }
}

fn lex(src: &str) -> Result<Vec<Token>, String> {
    let c: Vec<char> = src.chars().collect();
    let n = c.len();
    let mut i = 0;
    let mut line = 1;
    let mut out = Vec::new();
    if c.starts_with(&['#', '!']) {
        // Shebang, якщо далі не атрибут `#![`.
        if c.get(skip_trivia(&c, 2)) != Some(&'[') {
            while i < n && c[i] != '\n' {
                i += 1;
            }
        }
    }
    let err = |line: usize| format!("лексична помилка біля рядка {line}");
    while i < n {
        let ch = c[i];
        let start_line = line;
        if ch == '\n' {
            line += 1;
            i += 1;
        } else if ch.is_whitespace() {
            i += 1;
        } else if ch == '/' && c.get(i + 1) == Some(&'/') {
            let doc = match (c.get(i + 2), c.get(i + 3)) {
                (Some('/'), Some('/')) => None,
                (Some('/'), _) => Some(Kind::DocOuter),
                (Some('!'), _) => Some(Kind::DocInner),
                _ => None,
            };
            while i < n && c[i] != '\n' {
                i += 1;
            }
            if let Some(kind) = doc {
                out.push(Token {
                    kind,
                    line: start_line,
                });
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            let doc = match (c.get(i + 2), c.get(i + 3)) {
                (Some('*'), Some('*' | '/')) => None,
                (Some('*'), _) => Some(Kind::DocOuter),
                (Some('!'), _) => Some(Kind::DocInner),
                _ => None,
            };
            let mut depth = 0;
            loop {
                if i >= n {
                    return Err(err(start_line));
                }
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if c[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            if let Some(kind) = doc {
                out.push(Token {
                    kind,
                    line: start_line,
                });
            }
        } else if is_ident_start(ch) {
            let mut j = i;
            while j < n && is_ident_continue(c[j]) {
                j += 1;
            }
            let word: String = c[i..j].iter().collect();
            let next = c.get(j).copied();
            let is_str_prefix = matches!(word.as_str(), "r" | "b" | "br" | "c" | "cr");
            if is_str_prefix && (next == Some('"') || (next == Some('#') && word.ends_with('r'))) {
                // Рядок із префіксом; `r#ident` обробляється нижче.
                let raw = word.ends_with('r');
                let mut k = j;
                let mut hashes = 0;
                if raw {
                    while c.get(k) == Some(&'#') {
                        hashes += 1;
                        k += 1;
                    }
                }
                if c.get(k) == Some(&'"') {
                    i = skip_string(&c, k, raw.then_some(hashes), &mut line)
                        .ok_or_else(|| err(start_line))?;
                    out.push(Token {
                        kind: Kind::Literal,
                        line: start_line,
                    });
                    continue;
                }
            }
            if word == "r" && next == Some('#') && c.get(j + 1).is_some_and(|&x| is_ident_start(x))
            {
                let mut k = j + 1;
                while k < n && is_ident_continue(c[k]) {
                    k += 1;
                }
                let raw_name: String = c[j + 1..k].iter().collect();
                out.push(Token {
                    kind: Kind::Ident(format!("r#{raw_name}")),
                    line: start_line,
                });
                i = k;
                continue;
            }
            if word == "b" && next == Some('\'') {
                i = skip_char(&c, j).ok_or_else(|| err(start_line))?;
                out.push(Token {
                    kind: Kind::Literal,
                    line: start_line,
                });
                continue;
            }
            out.push(Token {
                kind: Kind::Ident(word),
                line: start_line,
            });
            i = j;
        } else if ch.is_ascii_digit() {
            while i < n && (is_ident_continue(c[i])) {
                i += 1;
            }
            out.push(Token {
                kind: Kind::Literal,
                line: start_line,
            });
        } else if ch == '"' {
            i = skip_string(&c, i, None, &mut line).ok_or_else(|| err(start_line))?;
            out.push(Token {
                kind: Kind::Literal,
                line: start_line,
            });
        } else if ch == '\'' {
            let is_char = c.get(i + 1) == Some(&'\\') || c.get(i + 2) == Some(&'\'');
            if is_char {
                i = skip_char(&c, i).ok_or_else(|| err(start_line))?;
                out.push(Token {
                    kind: Kind::Literal,
                    line: start_line,
                });
            } else {
                // Лайфтайм: апостроф і ідентифікатор.
                i += 1;
                while i < n && is_ident_continue(c[i]) {
                    i += 1;
                }
                out.push(Token {
                    kind: Kind::Punct('\''),
                    line: start_line,
                });
            }
        } else {
            let kind = match ch {
                '(' | '[' | '{' => Kind::Open(ch),
                ')' | ']' | '}' => Kind::Close,
                _ => Kind::Punct(ch),
            };
            out.push(Token {
                kind,
                line: start_line,
            });
            i += 1;
        }
    }
    Ok(out)
}

/// `at` вказує на відкривальну лапку; повертає позицію після рядка.
fn skip_string(
    c: &[char],
    at: usize,
    raw_hashes: Option<usize>,
    line: &mut usize,
) -> Option<usize> {
    let mut i = at + 1;
    loop {
        let ch = *c.get(i)?;
        if ch == '\n' {
            *line += 1;
        }
        match raw_hashes {
            None if ch == '\\' => {
                if c.get(i + 1) == Some(&'\n') {
                    *line += 1;
                }
                i += 2;
                continue;
            }
            None if ch == '"' => return Some(i + 1),
            Some(h) if ch == '"' && (1..=h).all(|k| c.get(i + k) == Some(&'#')) => {
                return Some(i + 1 + h);
            }
            _ => {}
        }
        i += 1;
    }
}

/// `at` вказує на відкривальний апостроф.
fn skip_char(c: &[char], at: usize) -> Option<usize> {
    let mut i = at + 1;
    loop {
        match *c.get(i)? {
            '\\' => i += 2,
            '\'' => return Some(i + 1),
            '\n' => return None,
            _ => i += 1,
        }
    }
}

/// Для кожної відкривальної дужки — індекс парної закривальної.
fn match_groups(tokens: &[Token]) -> Result<Vec<usize>, String> {
    let mut close_of = vec![usize::MAX; tokens.len()];
    let mut stack = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        match t.kind {
            Kind::Open(_) => stack.push(i),
            Kind::Close => {
                let open = stack.pop().ok_or("незбалансовані дужки")?;
                close_of[open] = i;
            }
            _ => {}
        }
    }
    if stack.is_empty() {
        Ok(close_of)
    } else {
        Err("незбалансовані дужки".into())
    }
}

fn is_ident(t: &Token, name: &str) -> bool {
    matches!(&t.kind, Kind::Ident(s) if s == name)
}

fn is_open(t: &Token, ch: char) -> bool {
    t.kind == Kind::Open(ch)
}

/// Пропускає елемент без розбору: до `;` на верхньому рівні або до першої групи `{}`.
fn skip_item(tokens: &[Token], close_of: &[usize], mut i: usize, end: usize) -> usize {
    while i < end {
        match tokens[i].kind {
            Kind::Open('{') => return close_of[i] + 1,
            Kind::Open(_) => i = close_of[i] + 1,
            Kind::Punct(';') => return i + 1,
            _ => i += 1,
        }
    }
    end
}

/// Обходить елементи діапазону токенів і рекурсивно входить у вбудовані `mod { .. }`.
///
/// Знахідка — звичайна `fn` без `async`, `const`, `unsafe` й `extern`, з єдиним зовнішнім атрибутом
/// `#[test]`, без doc-коментаря й з порожнім тілом `{}`. До `found` потрапляє рядок ключового слова
/// `fn`.
fn walk_items(
    tokens: &[Token],
    close_of: &[usize],
    mut i: usize,
    end: usize,
    found: &mut Vec<usize>,
) {
    while i < end {
        let item_start = i;
        // Зовнішні атрибути; внутрішні (`#![..]`, `//!`) не рахуються.
        let mut outer_attrs: Vec<(usize, usize)> = Vec::new();
        let mut has_doc = false;
        loop {
            match tokens.get(i).map(|t| &t.kind) {
                Some(Kind::DocInner) if i < end => i += 1,
                Some(Kind::DocOuter) if i < end => {
                    has_doc = true;
                    i += 1;
                }
                Some(Kind::Punct('#')) if i < end => {
                    let inner = tokens
                        .get(i + 1)
                        .is_some_and(|t| t.kind == Kind::Punct('!'));
                    let br = i + 1 + usize::from(inner);
                    if tokens.get(br).is_some_and(|t| is_open(t, '[')) {
                        if !inner {
                            outer_attrs.push((br, close_of[br]));
                        }
                        i = close_of[br] + 1;
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        if i >= end {
            return;
        }
        if is_ident(&tokens[i], "pub") {
            i += 1;
            if tokens.get(i).is_some_and(|t| is_open(t, '(')) && i < end {
                i = close_of[i] + 1;
            }
        }
        let mut plain = true;
        while i < end {
            if is_ident(&tokens[i], "extern") {
                plain = false;
                i += 1;
                if tokens.get(i).is_some_and(|t| t.kind == Kind::Literal) {
                    i += 1;
                }
            } else if ["async", "const", "unsafe"].iter().any(|q| is_ident(&tokens[i], q))
                && tokens.get(i + 1).is_some_and(|t| is_ident(t, "fn") || matches!(&t.kind, Kind::Ident(s) if matches!(s.as_str(), "async" | "const" | "unsafe" | "extern")))
            {
                plain = false;
                i += 1;
            } else {
                break;
            }
        }
        if i >= end {
            return;
        }
        if is_ident(&tokens[i], "fn") {
            let fn_idx = i;
            // Пошук тіла: групи пропускаємо, `{` у кутових дужках (const generics) — теж.
            let mut depth = 0usize;
            let mut j = i + 1;
            let mut body = None;
            while j < end {
                match &tokens[j].kind {
                    // `m! { .. }` у типі повернення — не тіло; `-> ! {` — тіло.
                    Kind::Open('{')
                        if depth == 0
                            && j >= 2
                            && tokens[j - 1].kind == Kind::Punct('!')
                            && matches!(tokens[j - 2].kind, Kind::Ident(_)) =>
                    {
                        j = close_of[j] + 1;
                        continue;
                    }
                    Kind::Open('{') if depth == 0 => {
                        body = Some(j);
                        break;
                    }
                    Kind::Open(_) => {
                        j = close_of[j] + 1;
                        continue;
                    }
                    Kind::Punct('<') => depth += 1,
                    Kind::Punct('>') if !matches!(tokens[j - 1].kind, Kind::Punct('-' | '=')) => {
                        depth = depth.saturating_sub(1)
                    }
                    Kind::Punct(';') if depth == 0 => break,
                    _ => {}
                }
                j += 1;
            }
            let Some(open) = body else {
                i = j + 1;
                continue;
            };
            let close = close_of[open];
            let is_test_attr = outer_attrs.len() == 1 && {
                let (o, c) = outer_attrs[0];
                c == o + 2 && is_ident(&tokens[o + 1], "test")
            };
            if plain && !has_doc && is_test_attr && close == open + 1 {
                found.push(tokens[fn_idx].line);
            }
            i = close + 1;
        } else if is_ident(&tokens[i], "mod") && tokens.get(i + 2).is_some_and(|t| is_open(t, '{'))
        {
            let open = i + 2;
            walk_items(tokens, close_of, open + 1, close_of[open], found);
            i = close_of[open] + 1;
        } else {
            i = skip_item(tokens, close_of, i.max(item_start), end);
        }
    }
}
