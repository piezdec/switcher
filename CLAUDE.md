# Switcher

Автопереключатель раскладки клавиатуры для Windows на Rust (аналог Punto Switcher).
План и статус: `docs/ROADMAP.md` (локальный файл, в git не хранится) — в начале сессии читать его, если он есть.

## Устройство

- `crates/layouts` — физическая модель клавиатуры: VK + Shift + Caps → символ в каждом языке. Раскладки — данные.
- `crates/langmodel` — символьная модель Kneser-Ney: обучение (`train.rs`), двоичный формат, запросы.
- `crates/engine` — ядро без WinAPI: буфер слова из нажатий (`Stroke`), классификация клавиш (`keys.rs`),
  конвейер решения (`detect.rs`: защита → короткие слова → длина → «настоящее слово» → баллы; плюс `decide_early`),
  баллы и коэффициенты `Params` (`score.rs`), встроенные модели и словари (`lexicon.rs`, `data/*.lm`, `data/*.fst`).
  Вход — `Engine::on_key(KeyInput, Option<Lang>)`, выход — `Option<Fix>` (стереть N, напечатать текст, хвост, сменить язык).
- `crates/platform-win` — хуки, `SendInput` с меткой `SIGNATURE`, раскладки окон, процесс окна, значок в трее.
- `crates/app` — бинарник `switcher`: `controller.rs` (поток хука, решает Pass/Swallow, режимы и профили),
  `worker.rs` (выполняет Fix и досылает перехваченные нажатия), `ui.rs` (скрытое окно, трей, меню), `profiles.rs`.
- `tools/trainer` — строит `crates/engine/data/*` из `corpora/` (качает `bash tools/fetch-corpora.sh`).
- `tools/eval` — меряет качество на фразах Tatoeba и `tools/eval/data/tech.txt`.

## Инварианты

- Процедура хука должна отвечать быстро: никаких блокирующих вызовов, COM, буфера обмена, OpenProcess — всё это в worker или по таймеру.
- Пока `Queue::pending`, хук глотает все нажатия и кладёт их в очередь (`Action::Replay`); ядро видит их сразу.
- Весь синтезированный ввод помечается `input::SIGNATURE`; хук такой ввод пропускает, не анализируя.
- Набранный текст никуда не пишется. Журнал работы (`diag!`, `switcher.log` в папке настроек) — только события
  и причины пауз, без слов; пути — через `diag::redact`. Сеть — только проверка обновлений на GitHub.
- Любое изменение распознавания (`detect.rs`, `score.rs`, `Params`, модели) — сверять с `tools/eval`:
  ложные срабатывания не должны расти. Текущие цифры — в `docs/ROADMAP.md`.

## Команды

- `cargo test --workspace` — тесты (сценарные тесты ядра в `crates/engine/src/tests.rs`)
- `cargo clippy --workspace --all-targets` — должно быть без предупреждений
- `cargo run --release -p eval` — качество; `-- --sweep` перебор порога; `-- --examples 20` примеры ошибок;
  `-- --explain ghbdtn` разбор решения по слову
- `cargo run --release -p trainer` — пересобрать модели (нужны корпуса)
- `cargo run -p switcher` — отладочный запуск с консолью для логов
- `cargo build --release` → `target/release/switcher.exe`

## Стиль

Комментарии и тексты интерфейса — по-русски. Новое поведение ядра — сначала сценарный тест в `tests.rs`.
В Bash-инструменте сложные многострочные правки делать через Edit, а не perl/sed (экранирование ломается).
