//! Методика `ts-loc` версії 1: кількість різних фізичних рядків, яких торкається хоча б
//! один токен коду (NCLOC). Коментарі, пробіли, BOM і shebang токенів коду не дають;
//! лексичний контекст (рядки, шаблони, регулярні вирази) визначає парсер.
//! Зміна правил підрахунку потребує нової версії.

use crate::ts_source::{ScoreError, line_of, line_starts, with_tokens};

/// Бал файла за його шляхом (лише для вибору режиму `.d.ts`) і сирим вмістом.
pub fn score(path: &[u8], content: &[u8]) -> Result<u64, ScoreError> {
    with_tokens(path, content, |text, tokens| {
        let starts = line_starts(text);
        let mut counted = 0u64;
        // Останній уже врахований рядок; токени йдуть у порядку джерела.
        let mut last: Option<usize> = None;
        for token in tokens {
            let (start, end) = (token.start() as usize, token.end() as usize);
            if end <= start {
                continue;
            }
            let first = line_of(&starts, start);
            let final_line = line_of(&starts, end - 1);
            let from = last.map_or(first, |l| first.max(l + 1));
            if final_line >= from {
                counted += (final_line - from + 1) as u64;
                last = Some(final_line);
            }
        }
        counted
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(src: &str) -> Result<u64, ScoreError> {
        score(b"a.ts", src.as_bytes())
    }

    #[test]
    fn empty_and_comment_only_files_are_zero() {
        for src in [
            "",
            "\n\n",
            "  \t\n \n",
            "// a\n/* b\n\n c */\n/** d\n * e\n */",
            "\u{feff}",
            "\u{feff}// a",
            "#!/usr/bin/env node\n",
        ] {
            assert_eq!(loc(src), Ok(0), "{src:?}");
        }
    }

    #[test]
    fn code_lines_blank_lines_and_bom_shebang() {
        assert_eq!(loc("let a = 1;\n\n\nlet b = 2;\n"), Ok(2));
        assert_eq!(loc("\u{feff}let a = 1;\n"), Ok(1));
        assert_eq!(loc("#!/usr/bin/env node\nlet a = 1;\n"), Ok(1));
        assert_eq!(loc("\u{feff}// c\n\nlet a = é;\nlet b;"), Ok(2));
    }

    #[test]
    fn several_tokens_on_one_line_count_once_and_punctuation_counts() {
        assert_eq!(loc("let a = 1; let b = 2; f(a, b);"), Ok(1));
        assert_eq!(loc("function f()\n{\n}\n"), Ok(3));
        assert_eq!(loc("f(\n)\n;\n"), Ok(3));
    }

    #[test]
    fn comments_share_lines_with_code() {
        assert_eq!(loc("let a = 1; // c\n"), Ok(1));
        assert_eq!(loc("/* c */ let a = 1;\n"), Ok(1));
        assert_eq!(loc("let a = /* c */ 1;\n"), Ok(1));
        assert_eq!(loc("/* a\n b */ let a = 1;\n"), Ok(1));
        assert_eq!(loc("let a = 1; /* a\n b\n c */\n"), Ok(1));
        assert_eq!(loc("let a = 1; /* a\n b */ let b = 2;\n"), Ok(2));
        assert_eq!(loc("/**\n * doc\n */\nlet a;\n"), Ok(1));
        assert_eq!(loc("let a;\n// x\n/* y */\nlet b;\n"), Ok(2));
    }

    #[test]
    fn comment_markers_inside_literals_are_code() {
        assert_eq!(loc("let a = \"//\";\nlet b = '/*';\n"), Ok(2));
        assert_eq!(loc("let a = `//`;\nlet b = `/*`;\n"), Ok(2));
        assert_eq!(loc("let a = /\\/\\//;\nlet b = /[/*]/;\n"), Ok(2));
        // Хибне читання `/*` у рядку «з'їло» б наступні рядки.
        assert_eq!(loc("let a = \"/*\";\nlet b = 1;\nlet c = \"*/\";\n"), Ok(3));
        assert_eq!(loc("let a = `/*`;\nlet b = 1;\nlet c = `*/`;\n"), Ok(3));
    }

    #[test]
    fn multiline_tokens_cover_all_lines() {
        assert_eq!(loc("let a = `x\n\ny`;\n"), Ok(3));
        assert_eq!(
            loc("let a = `x\n// not comment\n/* nor this */\ny`;\n"),
            Ok(4)
        );
        assert_eq!(loc("let a = 'x\\\n\\\ny';\n"), Ok(3));
        assert_eq!(loc("let a = 'x\\\n// z\\\ny';\n"), Ok(3));
    }

    #[test]
    fn template_substitution_uses_code_and_comment_rules() {
        assert_eq!(loc("let a = `x${\n// c\n1\n}y`;\n"), Ok(3));
        assert_eq!(loc("let a = `x${\n/* c\n d */\n1}y`;\n"), Ok(2));
        assert_eq!(loc("let a = `x${1 /* c */}y`;\n"), Ok(1));
        assert_eq!(loc("let a = `x${\n1\n// c\n}`;\n"), Ok(3));
    }

    #[test]
    fn line_terminators() {
        for t in ["\n", "\r", "\r\n", "\u{2028}", "\u{2029}"] {
            assert_eq!(
                loc(&format!("let a;{t}{t}let b;{t}// c{t}let c;")),
                Ok(3),
                "{t:?}"
            );
            assert_eq!(loc(&format!("// c{t}let a;{t}")), Ok(1), "{t:?}");
            assert_eq!(loc(&format!("let a = `x{t}{t}y`;")), Ok(3), "{t:?}");
        }
        // CRLF — один кінець, а не два.
        assert_eq!(loc("let a;\r\n\r\nlet b;\r\n"), Ok(2));
        assert_eq!(loc("let a = `x\r\ny`;\r\nlet b;"), Ok(3));
        assert_eq!(loc("let a;\r\n// c\r\nlet b;"), Ok(2));
        // Одинокий CR і LF поруч дають окремі кінці.
        assert_eq!(loc("let a;\n\rlet b;"), Ok(2));
    }

    #[test]
    fn multibyte_text_keeps_offsets_aligned() {
        assert_eq!(
            loc("let s = \"日本語\"; let t = \"é\";\n// ü\nlet u;\n"),
            Ok(2)
        );
        assert_eq!(loc("\u{feff}let s = \"😀\";\n// x\nlet t;\n"), Ok(2));
    }

    #[test]
    fn last_line_without_terminator_counts() {
        assert_eq!(loc("let a;"), Ok(1));
        assert_eq!(loc("let a;\nlet b;"), Ok(2));
    }

    #[test]
    fn declarations_are_code() {
        let src = "import { a } from 'a';\ninterface I {\n  x: number;\n}\ntype T = I | null;\ndeclare const c: T;\n";
        assert_eq!(loc(src), Ok(6));
        assert_eq!(
            score(
                b"a.d.ts",
                b"declare function f(): void;\n// c\ndeclare const x: number;\n"
            ),
            Ok(2)
        );
    }

    #[test]
    fn errors_give_no_partial_score() {
        assert_eq!(
            score(b"a.ts", b"let a;\n\xff"),
            Err(ScoreError::InvalidUtf8)
        );
        assert_eq!(
            score(b"a.ts", b"let a = 1;\nlet b = 2;\nfunction ("),
            Err(ScoreError::Syntax)
        );
        assert_eq!(
            score(b"a.ts", b"\xef\xbb\xbflet a;\nlet = = ;\n"),
            Err(ScoreError::Syntax)
        );
        // Синтаксична діагностика за відновленого дерева.
        assert_eq!(loc("let a = 1;\nlet a = ;\n"), Err(ScoreError::Syntax));
    }

    #[test]
    fn corpus_matches_expected_tsv() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts-loc");
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
