# Корпус «Порожній тест»

Десять автономних прикладів у п'яти парах для правила `empty-test`: три пари Rust і
дві пари TypeScript.

Rust: знахідка — звичайна функція з єдиним `#[test]`, тіло якої містить лише пробіли та
звичайні коментарі. Виклик допоміжної функції — виконуваний вміст, навіть якщо перевірка
лежить усередині неї. `#[ignore]`, `#[should_panic]`, умовна компіляція й тести з макросів
поза межами першої версії.

TypeScript: знахідка — прямий виклик `it` чи `test` з рядковим першим аргументом і
функцією з порожнім блоком (лише пробіли й коментарі) другим аргументом.

| Пара | Знахідка | Чистий | Визначальна зміна |
| --- | --- | --- | --- |
| `empty-body` | `empty_body_finding.rs` | `empty_body_clean.rs` | У порожнє тіло додано `assert_eq!(add(2, 3), 5)`. |
| `comment-only` | `comment_only_finding.rs` | `comment_only_clean.rs` | Єдиний коментар у тілі замінено викликом `check_sum()`; `check_sum` однакова в обох файлах. |
| `no-test-attr` | `no_test_attr_finding.rs` | `no_test_attr_clean.rs` | Вилучено лише рядок `#[test]`. |
| `ts-direct-call` | `ts_direct_call_finding.ts` | `ts_direct_call_clean.ts` | Прямий `it(...)` замінено викликом властивості `it.todo(...)`. |
| `ts-comment-only` | `ts_comment_only_finding.ts` | `ts_comment_only_clean.ts` | Єдиний коментар у тілі замінено інструкцією `add(2, 3);`. |

## expected.tsv

UTF-8, поля через табуляцію: `path`, `pair`, `lines`. `path` — унікальний шлях
від каталогу корпусу; `pair` — ідентифікатор пари; `lines` — `[N]` для знахідки або `[]`
для чистого файла. N — номер фізичного рядка (від 1): для Rust — ключового слова `fn`
тесту, для TypeScript — ідентифікатора `it` чи `test`. Обидва файли пари — однієї мови.

## Перевірка (з кореня проєкту, без мережі)

```sh
T=$(mktemp -d)
rustc tests/fixtures/check_corpus.rs -o "$T/check_corpus"
"$T/check_corpus" tests/fixtures/empty-test             # код 0

for f in tests/fixtures/empty-test/*.rs; do
  rustc --test "$f" -o "$T/$(basename "$f" .rs)" || echo "ПОМИЛКА $f"
done                                                    # у файлі без #[test] можливе попередження

cp -r tests/fixtures/empty-test "$T/copy"
rm "$T/copy/ts_direct_call_clean.ts"
"$T/check_corpus" "$T/copy"                             # ненульовий код, називає відсутній файл
```

Перевірник структурний: відповідність `[N]` позиції і зміст тіла звіряють вручну, а
поведінку бінарника — тест `corpus_matches_expected_tsv` у `tests/cli.rs`.
