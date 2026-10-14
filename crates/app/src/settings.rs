//! Окно настроек — отдельный процесс (`switcher.exe --settings`).
//!
//! Окно правит `config.toml` и `learned.toml`, работающий Switcher подхватывает изменения сам.
//! Перед каждой правкой файл перечитывается: пока окно открыто, основная программа могла
//! выучить новые слова, и затирать их нельзя.

use crate::config::{self, Config, ModeName, TranslitStyle};
use crate::hotkeys::{key_name, Hotkey};
use crate::profiles;
use eframe::egui;
use engine::learning::{self, Entry};
use engine::{Lang, Learning, Snippet};

pub const TITLE: &str = "Switcher — настройки";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Hotkeys,
    Apps,
    Dictionary,
    Snippets,
    About,
}

/// Какую горячую клавишу сейчас записываем.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HotkeySlot {
    ConvertWord,
    ConvertSelection,
    Transliterate,
    ToggleAuto,
}

impl HotkeySlot {
    fn get(self, cfg: &mut Config) -> &mut String {
        match self {
            HotkeySlot::ConvertWord => &mut cfg.hotkeys.convert_word,
            HotkeySlot::ConvertSelection => &mut cfg.hotkeys.convert_selection,
            HotkeySlot::ToggleAuto => &mut cfg.hotkeys.toggle_auto,
            HotkeySlot::Transliterate => &mut cfg.hotkeys.transliterate_selection,
        }
    }
}

struct Settings {
    tab: Tab,
    config: Config,
    learning: Learning,
    snippets: Vec<Snippet>,
    autostart: bool,
    recording: Option<HotkeySlot>,
    capture: Capture,
    new_app: String,
    new_word: String,
    filter: String,
    message: Option<String>,
    stamps: (Option<std::time::SystemTime>, Option<std::time::SystemTime>, Option<std::time::SystemTime>),
}

/// `tab` — с какой вкладки открыть: general, hotkeys, apps, dictionary, snippets, about.
pub fn run(tab: Option<&str>) -> eframe::Result<()> {
    let tab = match tab {
        Some("hotkeys") => Tab::Hotkeys,
        Some("apps") => Tab::Apps,
        Some("dictionary") => Tab::Dictionary,
        Some("snippets") => Tab::Snippets,
        Some("about") => Tab::About,
        _ => Tab::General,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_inner_size([720.0, 540.0])
            .with_min_inner_size([560.0, 420.0])
            .with_icon(window_icon())
            // Класс окна в Linux: по нему основная программа узнаёт своё окно и не трогает его.
            .with_app_id("switcher"),
        ..Default::default()
    };
    eframe::run_native(
        TITLE,
        options,
        Box::new(move |_cc| Ok(Box::new(Settings { tab, ..Settings::load() }))),
    )
}

/// Оранжевый квадрат, как значок в трее.
fn window_icon() -> egui::IconData {
    let size = 32;
    let rgba = (0..size * size).flat_map(|_| [0xF2, 0x8C, 0x28, 0xFF]).collect();
    egui::IconData { rgba, width: size as u32, height: size as u32 }
}

impl Settings {
    fn load() -> Self {
        Settings {
            tab: Tab::General,
            config: config::load_config(),
            learning: config::load_learning(),
            snippets: config::load_snippets(),
            autostart: crate::platform::autostart_enabled(),
            recording: None,
            capture: Capture::start(),
            new_app: String::new(),
            new_word: String::new(),
            filter: String::new(),
            message: None,
            stamps: (
                config::modified(&config::config_path()),
                config::modified(&config::learned_path()),
                config::modified(&config::snippets_path()),
            ),
        }
    }

    /// Подтягивает изменения файлов, сделанные основной программой или руками.
    fn refresh_from_disk(&mut self) {
        let config_stamp = config::modified(&config::config_path());
        if config_stamp != self.stamps.0 {
            self.stamps.0 = config_stamp;
            self.config = config::load_config();
        }
        let learned_stamp = config::modified(&config::learned_path());
        if learned_stamp != self.stamps.1 {
            self.stamps.1 = learned_stamp;
            self.learning = config::load_learning();
        }
        let snippets_stamp = config::modified(&config::snippets_path());
        if snippets_stamp != self.stamps.2 {
            self.stamps.2 = snippets_stamp;
            self.snippets = config::load_snippets();
        }
    }

    fn save_config(&mut self, f: impl FnOnce(&mut Config)) {
        self.refresh_from_disk();
        f(&mut self.config);
        let path = config::config_path();
        if let Err(e) = config::write_file(&path, &config::config_to_string(&self.config)) {
            self.message = Some(format!("Не удалось сохранить настройки: {e}"));
        }
        self.stamps.0 = config::modified(&path);
    }

    fn save_snippets(&mut self, f: impl FnOnce(&mut Vec<Snippet>)) {
        self.refresh_from_disk();
        f(&mut self.snippets);
        let path = config::snippets_path();
        if let Err(e) = config::write_file(&path, &config::snippets_to_string(&self.snippets)) {
            self.message = Some(format!("Не удалось сохранить сокращения: {e}"));
        }
        self.stamps.2 = config::modified(&path);
    }

    fn save_learning(&mut self, f: impl FnOnce(&mut Learning)) {
        self.refresh_from_disk();
        f(&mut self.learning);
        let path = config::learned_path();
        if let Err(e) = config::write_file(&path, &config::learning_to_string(&self.learning)) {
            self.message = Some(format!("Не удалось сохранить словарь: {e}"));
        }
        self.stamps.1 = config::modified(&path);
    }

    fn general(&mut self, ui: &mut egui::Ui) {
        let mut cfg = self.config.clone();
        let mut changed = false;

        changed |= ui.checkbox(&mut cfg.auto, "Автоматическое переключение").changed();
        ui.label(egui::RichText::new("Ручной перевод горячей клавишей работает всегда.").weak());
        ui.add_space(12.0);

        ui.strong("Чувствительность");
        for (mode, title, hint) in [
            (ModeName::Careful, "Осторожно", "Только явные ошибки. Меньше всего лишних срабатываний."),
            (ModeName::Normal, "Обычно", "Рекомендуется. Подобрано по проверке на тысячах фраз."),
            (ModeName::Aggressive, "Активно", "Исправляет больше, но иногда срабатывает зря."),
        ] {
            ui.horizontal(|ui| {
                changed |= ui.radio_value(&mut cfg.mode, mode, title).changed();
                ui.label(egui::RichText::new(hint).weak());
            });
        }
        ui.add_space(8.0);
        changed |= ui
            .checkbox(&mut cfg.careful_in_dev_tools, "Осторожно в терминалах и редакторах кода")
            .on_hover_text("Windows Terminal, PowerShell, VS Code, JetBrains IDE и другие — список на вкладке «Исключения».")
            .changed();
        changed |= ui
            .checkbox(&mut cfg.pause_in_fullscreen, "Не работать в играх (на весь экран или из Steam, Epic, GOG и других лаунчеров)")
            .on_hover_text(
                "Игру в окне Switcher узнаёт по папке, куда её поставил лаунчер. Любую другую можно исключить из меню значка.",
            )
            .changed();
        changed |= ui
            .checkbox(&mut cfg.auto_update, "Обновляться автоматически")
            .on_hover_text(
                "Раз в несколько часов проверяет страницу релизов на GitHub и ставит новую версию. \
                 Это единственное, ради чего программа выходит в сеть.",
            )
            .changed();
        ui.add_space(12.0);
        ui.strong("Вид и звук");
        changed |= ui.checkbox(&mut cfg.indicator, "Плашка с раскладкой у курсора при смене языка").changed();
        changed |= ui
            .checkbox(&mut cfg.switch_sound, "Звук при смене раскладки")
            .on_hover_text(
                "Свой звук: положите switch.wav в папку настроек (вкладка «О программе» → «Открыть папку»).",
            )
            .changed();
        changed |= ui
            .checkbox(&mut cfg.typing_click, "Тихий щелчок клавиш при наборе по-русски")
            .on_hover_text("Чтобы на слух отличать, в какой раскладке печатаете.")
            .changed();
        ui.add_space(12.0);
        ui.strong("Исправление текста");
        changed |= ui
            .checkbox(
                &mut cfg.fix_case,
                "Исправлять регистр: ПРивет и пРИВЕТ станут Привет (и выключать случайный Caps Lock)",
            )
            .changed();
        changed |= ui
            .checkbox(&mut cfg.capitalize_sentences, "Заглавная буква в начале предложения")
            .on_hover_text("Кроме терминалов и редакторов кода.")
            .changed();
        changed |=
            ui.checkbox(&mut cfg.fix_typos, "Исправлять частые опечатки: собщение, teh, recieve…").changed();
        ui.horizontal(|ui| {
            ui.label("Транслитерация:");
            changed |= ui
                .radio_value(&mut cfg.translit_style, TranslitStyle::Simple, "как в интернете (Schuka)")
                .changed();
            changed |= ui
                .radio_value(
                    &mut cfg.translit_style,
                    TranslitStyle::Passport,
                    "как в загранпаспорте (Shchuka)",
                )
                .changed();
        });
        if changed {
            self.save_config(|c| *c = cfg);
        }

        ui.add_space(12.0);
        ui.separator();
        let mut autostart_on = self.autostart;
        if ui
            .checkbox(
                &mut autostart_on,
                if cfg!(windows) {
                    "Запускать вместе с Windows (скрыто, только значок в трее)"
                } else {
                    "Запускать при входе в систему (скрыто, только значок в трее)"
                },
            )
            .changed()
        {
            if crate::platform::set_autostart(autostart_on) {
                self.autostart = autostart_on;
            } else {
                self.message = Some("Не удалось изменить автозапуск.".into());
            }
        }
    }

    fn hotkeys(&mut self, ui: &mut egui::Ui) {
        ui.label("Выберите сочетание из списка — оно сразу сохраняется.");
        ui.label("Чтобы задать своё, выберите «Своё сочетание…» и нажмите нужные клавиши.");
        ui.add_space(10.0);

        // Запись своего сочетания: ловим нажатие напрямую через Windows — так видны все клавиши,
        // включая Pause и Scroll Lock, которых нет в egui.
        if let Some(slot) = self.recording {
            ui.ctx().request_repaint();
            // Где опроса клавиш нет (Linux), сочетание берём из событий окна.
            let captured = if crate::platform::key_down(0x10).is_some() {
                self.capture.poll()
            } else {
                egui_capture(ui)
            };
            match captured {
                Some(Captured::Cancel) => self.recording = None,
                Some(Captured::Combo(text)) => {
                    self.recording = None;
                    self.save_config(|c| *slot.get(c) = text);
                }
                Some(Captured::Unsupported) => {
                    self.recording = None;
                    self.message = Some("Эту клавишу назначить нельзя — выберите другую.".into());
                }
                None => {}
            }
        }

        egui::Grid::new("hotkeys").num_columns(2).spacing([24.0, 12.0]).show(ui, |ui| {
            for (slot, title) in [
                (HotkeySlot::ConvertWord, "Перевести последнее слово"),
                (HotkeySlot::ConvertSelection, "Перевести выделенный текст"),
                (HotkeySlot::Transliterate, "Транслитерировать выделенное"),
                (HotkeySlot::ToggleAuto, "Включить / выключить"),
            ] {
                ui.label(title);
                if self.recording == Some(slot) {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Нажмите сочетание клавиш…")
                                .strong()
                                .color(egui::Color32::from_rgb(0xF2, 0x8C, 0x28)),
                        );
                        if ui.small_button("Отмена").clicked() {
                            self.recording = None;
                        }
                    });
                    ui.end_row();
                    continue;
                }
                let mut cfg = self.config.clone();
                let current = slot.get(&mut cfg).clone();
                let shown =
                    if current.is_empty() { "не назначено".to_string() } else { current.clone() };
                egui::ComboBox::from_id_salt(title).width(200.0).selected_text(shown).show_ui(ui, |ui| {
                    for preset in
                        ["Pause", "Shift+Pause", "Ctrl+Pause", "DoubleShift", "DoubleCtrl", "ScrollLock"]
                    {
                        if ui.selectable_label(current == preset, preset).clicked() {
                            self.save_config(|c| *slot.get(c) = preset.to_string());
                        }
                    }
                    ui.separator();
                    if ui.selectable_label(false, "Своё сочетание…").clicked() {
                        self.capture = Capture::start();
                        self.recording = Some(slot);
                    }
                    if ui.selectable_label(current.is_empty(), "Не назначено").clicked() {
                        self.save_config(|c| slot.get(c).clear());
                    }
                });
                ui.end_row();
            }
        });
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("DoubleShift — два быстрых нажатия Shift подряд, без других клавиш.").weak(),
        );
    }

    fn apps(&mut self, ui: &mut egui::Ui) {
        ui.label("В этих программах переключатель ничего не делает. Имя — как у exe-файла.");
        ui.add_space(8.0);
        let mut remove = None;
        for (i, app) in self.config.excluded_apps.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("✖").on_hover_text("Убрать из исключений").clicked() {
                    remove = Some(i);
                }
                ui.label(app);
            });
        }
        if self.config.excluded_apps.is_empty() {
            ui.label(
                egui::RichText::new("Пока пусто. Программу можно добавить и из меню значка в трее.").weak(),
            );
        }
        if let Some(i) = remove {
            self.save_config(|c| {
                c.excluded_apps.remove(i);
            });
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_app).hint_text("например, keepass.exe"));
            let app = self.new_app.trim().to_lowercase();
            if ui.add_enabled(!app.is_empty(), egui::Button::new("Добавить")).clicked() {
                let app = if app.ends_with(".exe") { app } else { format!("{app}.exe") };
                self.save_config(|c| {
                    if !c.excluded_apps.contains(&app) {
                        c.excluded_apps.push(app);
                    }
                });
                self.new_app.clear();
            }
        });

        ui.add_space(16.0);
        ui.collapsing(
            "Где включается осторожный режим (терминалы и редакторы кода)",
            |ui| {
                ui.label(profiles::DEV_TOOLS.join(", "));
            },
        );
    }

    fn dictionary(&mut self, ui: &mut egui::Ui) {
        ui.label(
            "Слова, которые Switcher выучил на ваших действиях или которые вы добавили сами. \
             Срабатывают раньше модели.",
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_word).hint_text("слово, как оно должно писаться"),
            );
            let parsed = learning::key_for_word(&self.new_word);
            if ui
                .add_enabled(parsed.is_some(), egui::Button::new("Всегда писать так"))
                .on_hover_text("«гига» — всегда по-русски, «xfl» — никогда не переводить")
                .clicked()
            {
                if let Some((key, lang)) = parsed {
                    self.save_learning(|l| l.set(key, Entry { lang, score: learning::MAX_SCORE }));
                    self.new_word.clear();
                }
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Поиск:");
            ui.text_edit_singleline(&mut self.filter);
        });
        ui.add_space(4.0);

        let filter = self.filter.trim().to_lowercase();
        let mut rows: Vec<(String, String, Entry)> = self
            .learning
            .entries()
            .map(|(k, e)| (layouts::convert_str(k, Lang::En, e.lang), k.clone(), *e))
            .filter(|(shown, key, _)| filter.is_empty() || shown.contains(&filter) || key.contains(&filter))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        let mut forget = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("words").num_columns(4).striped(true).spacing([20.0, 6.0]).show(ui, |ui| {
                ui.strong("Всегда пишется так");
                ui.strong("Вместо");
                ui.strong("Действует");
                ui.label("");
                ui.end_row();
                for (shown, key, e) in &rows {
                    ui.label(shown);
                    ui.label(egui::RichText::new(layouts::convert_str(shown, e.lang, e.lang.other())).weak());
                    ui.label(if e.score >= learning::THRESHOLD {
                        "да".to_string()
                    } else {
                        format!("ещё {} раз", learning::THRESHOLD - e.score)
                    });
                    if ui.small_button("Забыть").clicked() {
                        forget = Some(key.clone());
                    }
                    ui.end_row();
                }
            });
            if rows.is_empty() {
                ui.label(
                    egui::RichText::new(
                        "Пусто. Нажимайте Pause на неверно переведённых словах — они появятся здесь.",
                    )
                    .weak(),
                );
            }
        });
        if let Some(key) = forget {
            self.save_learning(|l| l.forget(&key));
        }
    }

    fn snippets_tab(&mut self, ui: &mut egui::Ui) {
        ui.label("Набрали сокращение и пробел — получили полный текст. Работает в любой раскладке.");
        ui.label(
            egui::RichText::new(
                "В тексте можно использовать {date} — дата, {time} — время, {clipboard} — буфер обмена, \
                 {cursor} — где оставить курсор.",
            )
            .weak(),
        );
        ui.add_space(8.0);
        if ui.button("➕  Добавить сокращение").clicked() {
            self.save_snippets(|s| s.push(Snippet { trigger: String::new(), text: String::new() }));
        }
        ui.add_space(8.0);

        let mut edited = None;
        let mut remove = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, snippet) in self.snippets.iter().enumerate() {
                let mut trigger = snippet.trigger.clone();
                let mut text = snippet.text.clone();
                ui.horizontal_top(|ui| {
                    let t = ui.add(
                        egui::TextEdit::singleline(&mut trigger).hint_text("сокращение").desired_width(110.0),
                    );
                    let x = ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .hint_text("полный текст")
                            .desired_rows(2)
                            .desired_width(ui.available_width() - 40.0),
                    );
                    if ui.small_button("✖").on_hover_text("Удалить").clicked() {
                        remove = Some(i);
                    }
                    if t.changed() || x.changed() {
                        edited = Some((i, Snippet { trigger, text }));
                    }
                });
                ui.add_space(6.0);
            }
            if self.snippets.is_empty() {
                ui.label(egui::RichText::new("Пока нет ни одного сокращения.").weak());
            }
        });
        if let Some((i, s)) = edited {
            self.save_snippets(|all| {
                if let Some(slot) = all.get_mut(i) {
                    *slot = s;
                }
            });
        }
        if let Some(i) = remove {
            self.save_snippets(|all| {
                if i < all.len() {
                    all.remove(i);
                }
            });
        }
    }

    /// Обновление в два шага: «Проверить» → «Доступна версия… [Обновить и перезапустить]».
    /// Сеть и установка — фоном, не в потоке окна.
    fn update_block(&mut self, ui: &mut egui::Ui) {
        use crate::update::Status;
        ui.horizontal(|ui| match crate::update::status() {
            Status::Idle | Status::UpToDate | Status::Failed(_) => {
                if ui.button("Проверить обновления").clicked() {
                    std::thread::spawn(crate::update::check);
                }
                let note = match crate::update::status() {
                    Status::UpToDate => "у вас последняя версия".to_string(),
                    Status::Failed(why) => format!("не получилось: {why}"),
                    _ => String::new(),
                };
                ui.label(egui::RichText::new(note).weak());
            }
            Status::Checking => {
                ui.label("Проверяю…");
            }
            Status::Available(release) => {
                ui.label(egui::RichText::new(format!("Доступна версия {}", release.tag)).strong());
                if ui.button("Обновить и перезапустить").clicked() {
                    std::thread::spawn(|| {
                        if matches!(crate::update::install_available(), Status::Ready { .. }) {
                            crate::update::restart_main_into_update();
                        }
                    });
                }
            }
            Status::Installing => {
                ui.label("Скачиваю и проверяю новую версию…");
            }
            Status::Ready { tag, .. } => {
                ui.label(format!("Готово: Switcher обновлён до {tag} и перезапущен."));
                if ui
                    .small_button("Перезапустить ещё раз")
                    .on_hover_text("Если значок в трее пропал")
                    .clicked()
                {
                    std::thread::spawn(crate::update::restart_main_into_update);
                }
            }
        });
        if matches!(crate::update::status(), Status::Checking | Status::Installing) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
        }
    }

    fn about(&mut self, ui: &mut egui::Ui) {
        ui.heading("Switcher");
        ui.label(format!("Версия {}", env!("CARGO_PKG_VERSION")));
        self.update_block(ui);
        ui.add_space(12.0);
        if ui
            .button("📂  Открыть папку с настройками и словарём")
            .on_hover_text("config.toml и learned.toml — их можно править и руками")
            .clicked()
        {
            let dir = config::data_dir();
            let _ = std::fs::create_dir_all(&dir);
            crate::platform::open_folder(&dir);
        }
        if ui
            .button("📄  Открыть журнал работы")
            .on_hover_text("Что делала программа и почему была на паузе — без набранного текста. Пришлите его, если что-то не так.")
            .clicked()
        {
            crate::platform::open_folder(&crate::diag::path());
        }
    }
}

impl eframe::App for Settings {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Файлы могут поменяться снаружи — проверяем раз в секунду, даже без действий пользователя.
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
        if self.recording.is_none() {
            self.refresh_from_disk();
        }

        egui::Panel::top("tabs").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for (tab, title) in [
                    (Tab::General, "Общее"),
                    (Tab::Hotkeys, "Горячие клавиши"),
                    (Tab::Apps, "Исключения"),
                    (Tab::Dictionary, "Словарь"),
                    (Tab::Snippets, "Сокращения"),
                    (Tab::About, "О программе"),
                ] {
                    ui.selectable_value(&mut self.tab, tab, title);
                }
            });
            ui.add_space(4.0);
        });
        if let Some(msg) = self.message.clone() {
            egui::Panel::bottom("message").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(0xE0, 0x60, 0x40), msg);
                    if ui.small_button("OK").clicked() {
                        self.message = None;
                    }
                });
            });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(8.0);
            match self.tab {
                Tab::General => self.general(ui),
                Tab::Hotkeys => self.hotkeys(ui),
                Tab::Apps => self.apps(ui),
                Tab::Dictionary => self.dictionary(ui),
                Tab::Snippets => self.snippets_tab(ui),
                Tab::About => self.about(ui),
            }
        });
    }
}

/// Результат записи сочетания.
enum Captured {
    Combo(String),
    Cancel,
    Unsupported,
}

/// Запись сочетания опросом состояния клавиш. Срабатывает на *нажатие*: клавиши, зажатые
/// в момент начала записи (например, после щелчка по пункту меню), не считаются.
struct Capture {
    was_down: Vec<bool>,
}

fn key_down(vk: u16) -> bool {
    crate::platform::key_down(vk).unwrap_or(false)
}

/// Модификаторы — частью сочетания, а не самостоятельной клавишей. Кнопки мыши (1–6) не берём.
fn is_modifier(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C)
}

impl Capture {
    fn start() -> Capture {
        Capture { was_down: (0..=0xFE).map(key_down).collect() }
    }

    fn poll(&mut self) -> Option<Captured> {
        let mut pressed = None;
        for vk in 0x08..=0xFEu16 {
            let down = key_down(vk);
            let fresh = down && !self.was_down[vk as usize];
            self.was_down[vk as usize] = down;
            if fresh && !is_modifier(vk) && pressed.is_none() {
                pressed = Some(vk);
            }
        }
        let vk = pressed?;
        if vk == 0x1B {
            return Some(Captured::Cancel);
        }
        if key_name(vk).is_none() {
            return Some(Captured::Unsupported);
        }
        let hotkey = Hotkey::Chord {
            ctrl: key_down(0x11),
            alt: key_down(0x12),
            shift: key_down(0x10),
            win: key_down(0x5B) || key_down(0x5C),
            vk,
        };
        Some(Captured::Combo(hotkey.to_string()))
    }
}

/// Запись сочетания из событий окна egui (там, где нельзя опросить клавиатуру напрямую).
fn egui_capture(ui: &egui::Ui) -> Option<Captured> {
    let (key, m) = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
            _ => None,
        })
    })?;
    if key == egui::Key::Escape {
        return Some(Captured::Cancel);
    }
    let mut text = String::new();
    for (on, name) in [(m.ctrl, "Ctrl+"), (m.alt, "Alt+"), (m.shift, "Shift+")] {
        if on {
            text.push_str(name);
        }
    }
    text.push_str(key.name());
    Some(match Hotkey::parse(&text) {
        Some(h) => Captured::Combo(h.to_string()),
        None => Captured::Unsupported,
    })
}
