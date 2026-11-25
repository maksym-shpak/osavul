# Корпус «Порожній Rust-тест»

Шість автономних прикладів у трьох парах. Знахідка — звичайна функція з єдиним
`#[test]`, тіло якої містить лише пробіли та звичайні коментарі. Виклик допоміжної
функції — виконуваний вміст, навіть якщо перевірка лежить усередині неї.
`#[ignore]`, `#[should_panic]`, умовна компіляція й тести з макросів поза межами
першої версії.

| Пара | Знахідка | Чистий | Визначальна зміна |
| --- | --- | --- | --- |
| `empty-body` | `empty_body_finding.rs` | `empty_body_clean.rs` | У порожнє тіло додано `assert_eq!(add(2, 3), 5)`. |
| `comment-only` | `comment_only_finding.rs` | `comment_only_clean.rs` | Єдиний коментар у тілі замінено викликом `check_sum()`; `check_sum` однакова в обох файлах. |
| `no-test-attr` | `no_test_attr_finding.rs` | `no_test_attr_clean.rs` | Вилучено лише рядок `#[test]`. |

## expected.tsv

UTF-8, поля через табуляцію: `path`, `pair`, `lines`. `path` — унікальний шлях
від каталогу корпусу; `pair` — ідентифікатор пари; `lines` — `[N]` для знахідки,
де N — номер фізичного рядка оголошення `fn` тесту (від 1), або `[]` для чистого файла.

## Перевірка (з кореня проєкту, без мережі)

```sh
T=$(mktemp -d)
rustc tests/fixtures/check_corpus.rs -o "$T/check_corpus"
"$T/check_corpus" tests/fixtures/empty-rust-test        # код 0

for f in tests/fixtures/empty-rust-test/*.rs; do
  rustc --test "$f" -o "$T/$(basename "$f" .rs)" || echo "ПОМИЛКА $f"
done                                                    # у файлі без #[test] можливе попередження

cp -r tests/fixtures/empty-rust-test "$T/copy"
rm "$T/copy/empty_body_clean.rs"
"$T/check_corpus" "$T/copy"                             # ненульовий код, називає відсутній файл
```

Перевірник структурний: відповідність `[N]` оголошенню `fn` і зміст тіла звіряють вручну.
