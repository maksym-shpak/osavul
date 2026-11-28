# Runbook: Реліз

Для того, хто випускає версію й ставить її на сервер. Рішення й межі — в [ADR-0004](../architecture/decisions/0004-ci-and-releases.md).

## Випуск

Тег дорівнює `v<версія з Cargo.toml>`, для пробного релізу — з суфіксом: `v0.1.0-rc.1` (такий реліз позначається як prerelease). Тег ставиться на коміт у `main`:

```sh
git switch main && git pull --ff-only
git tag v0.1.0-rc.1
git push origin v0.1.0-rc.1
```

Ім'я тегу опублікованого релізу вдруге не використати, тож помилку виправляє наступна версія. Якщо запуск упав до публікації, спершу перевірте, чи не лишилася чернетка релізу.

## Перевірка артефакту

У порожньому каталозі, де є `gh`:

```sh
gh release download v0.1.0-rc.1 --repo maksym-shpak/osavul
sha256sum -c SHA256SUMS
gh attestation verify osavul-*.tar.gz --repo maksym-shpak/osavul
tar -xzf osavul-*.tar.gz
```

`SHA256SUMS` виявляє пошкоджене завантаження, attestation підтверджує, що архів зібрано workflow цього репозиторію. Для `complexity`, `duplication` і `coverage` на машині досить бінарника (для ревізій — ще Git); `check` потребує `rustc` 1.100.0.
