//! Настройки и выученные слова на диске.
//!
//! Где лежат: `%APPDATA%\Switcher\` в Windows и `~/.config/switcher` в Linux, а если рядом
//! с программой есть `config.toml` — рядом с ней (портативный режим, удобно носить на флешке).
//!
//! - `config.toml` — настройки, их можно править руками;
//! - `learned.toml` — выученные слова: клавиши в английской раскладке = язык и уверенность.

use crate::hotkeys::HotkeyConfig;
use engine::learning::Entry;
use engine::translit::Style;
use engine::{Lang, Learning, Mode, Snippet};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModeName {
    Careful,
    Normal,
    Aggressive,
}

impl From<ModeName> for Mode {
    fn from(m: ModeName) -> Mode {
        match m {
            ModeName::Careful => Mode::Careful,
            ModeName::Normal => Mode::Normal,
            ModeName::Aggressive => Mode::Aggressive,
        }
    }
}

impl From<Mode> for ModeName {
    fn from(m: Mode) -> ModeName {
        match m {
            Mode::Careful => ModeName::Careful,
            Mode::Normal => ModeName::Normal,
            Mode::Aggressive => ModeName::Aggressive,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TranslitStyle {
    Simple,
    Passport,
}

impl From<TranslitStyle> for Style {
    fn from(s: TranslitStyle) -> Style {
        match s {
            TranslitStyle::Simple => Style::Simple,
            TranslitStyle::Passport => Style::Passport,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Автоматическое переключение (ручной перевод по Pause работает всегда).
    pub auto: bool,
    /// Чувствительность: careful, normal, aggressive.
    pub mode: ModeName,
    /// Осторожный режим в терминалах и редакторах кода.
    pub careful_in_dev_tools: bool,
    /// Программы (имя exe в нижнем регистре), где переключатель ничего не делает.
    pub excluded_apps: Vec<String>,
    /// Плашка с раскладкой у курсора при смене языка.
    pub indicator: bool,
    /// Звук при смене раскладки.
    pub switch_sound: bool,
    /// Тихий щелчок клавиш при наборе в русской раскладке.
    pub typing_click: bool,
    /// Исправлять ПРивет и пРИВЕТ (случайный Caps Lock).
    pub fix_case: bool,
    /// Заглавная буква в начале предложения.
    pub capitalize_sentences: bool,
    /// Исправлять частые опечатки.
    pub fix_typos: bool,
    /// Схема транслитерации выделенного текста.
    pub translit_style: TranslitStyle,
    /// Не работать, пока активная программа на весь экран (игры, видео).
    pub pause_in_fullscreen: bool,
    /// Проверять и ставить обновления с GitHub (единственное обращение в сеть).
    pub auto_update: bool,
    /// Горячие клавиши.
    pub hotkeys: HotkeyConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            auto: true,
            mode: ModeName::Normal,
            careful_in_dev_tools: true,
            excluded_apps: Vec::new(),
            indicator: true,
            switch_sound: false,
            typing_click: false,
            fix_case: true,
            capitalize_sentences: false,
            fix_typos: false,
            translit_style: TranslitStyle::Simple,
            pause_in_fullscreen: true,
            auto_update: true,
            hotkeys: HotkeyConfig::default(),
        }
    }
}

const CONFIG_HEADER: &str = "\
# Настройки Switcher. Файл можно править руками — работающая программа подхватит изменения сама.
#
# auto                  — автоматическое переключение (Pause работает всегда)
# mode                  — чувствительность: careful | normal | aggressive
# careful_in_dev_tools  — осторожный режим в терминалах и редакторах кода
# excluded_apps         — программы (имя exe), где переключатель ничего не делает
# auto_update           — проверять и ставить обновления с GitHub (единственное обращение в сеть)
# pause_in_fullscreen   — не работать в полноэкранных программах (игры, видео)
# indicator             — плашка с раскладкой у курсора при смене языка
# switch_sound          — звук при смене раскладки
# typing_click          — тихий щелчок клавиш при наборе по-русски
# fix_case              — исправлять ПРивет и пРИВЕТ (случайный Caps Lock)
# capitalize_sentences  — заглавная буква в начале предложения
# fix_typos             — исправлять частые опечатки (собщение → сообщение)
# translit_style        — транслитерация: simple (как в интернете) | passport (как в загранпаспорте)
# [hotkeys]             — горячие клавиши: «Pause», «Shift+Pause», «Ctrl+Alt+Space», «DoubleShift»;
#                         пустая строка — не назначено

";

/// Каталог данных: рядом с программой в портативном режиме, иначе `%APPDATA%\Switcher`
/// в Windows и `~/.config/switcher` в Linux.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        if dir.join("config.toml").exists() {
            return dir;
        }
    }
    if cfg!(windows) {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join("Switcher")
    } else {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("switcher")
    }
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

pub fn snippets_path() -> PathBuf {
    data_dir().join("snippets.toml")
}

pub fn learned_path() -> PathBuf {
    data_dir().join("learned.toml")
}

/// Читает настройки; если файла нет или он испорчен — настройки по умолчанию.
pub fn load_config() -> Config {
    std::fs::read_to_string(config_path()).ok().and_then(|s| parse_config(&s)).unwrap_or_default()
}

pub fn parse_config(text: &str) -> Option<Config> {
    toml::from_str(text).map_err(|e| debug_log!("config.toml не читается: {e}")).ok()
}

pub fn config_to_string(c: &Config) -> String {
    format!("{CONFIG_HEADER}{}", toml::to_string(c).unwrap_or_default())
}

#[derive(Deserialize)]
struct LearnedFile {
    #[serde(default)]
    words: BTreeMap<String, LearnedEntry>,
}

#[derive(Deserialize)]
struct LearnedEntry {
    lang: String,
    score: u8,
}

fn lang_from(code: &str) -> Option<Lang> {
    Lang::ALL.into_iter().find(|l| l.code().eq_ignore_ascii_case(code))
}

pub fn load_learning() -> Learning {
    std::fs::read_to_string(learned_path()).ok().and_then(|s| parse_learning(&s)).unwrap_or_default()
}

pub fn parse_learning(text: &str) -> Option<Learning> {
    let file: LearnedFile =
        toml::from_str(text).map_err(|e| debug_log!("learned.toml не читается: {e}")).ok()?;
    Some(Learning::from_entries(
        file.words
            .into_iter()
            .filter_map(|(k, e)| Some((k, Entry { lang: lang_from(&e.lang)?, score: e.score }))),
    ))
}

/// Время изменения файла — для подхвата правок на лету.
pub fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Выученные слова в TOML; в комментарии — как слово выглядит в выученном языке.
pub fn learning_to_string(l: &Learning) -> String {
    let mut out = String::from(
        "# Выученные слова: клавиши (как в английской раскладке) = в каком языке оставлять и уверенность.\n\
         # Срабатывает при score >= 2. Строку можно удалить, чтобы забыть слово.\n\n[words]\n",
    );
    let mut entries: Vec<_> = l.entries().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (key, e) in entries {
        let shown = layouts::convert_str(key, Lang::En, e.lang);
        let quoted = toml::Value::String(key.clone()).to_string();
        let _ =
            writeln!(out, "{quoted} = {{ lang = \"{}\", score = {} }}  # {shown}", e.lang.code(), e.score);
    }
    out
}

/// Атомарная запись: во временный файл, потом переименование, чтобы не оставить полфайла.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct SnippetsFile {
    #[serde(default, rename = "snippet")]
    snippets: Vec<SnippetEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SnippetEntry {
    trigger: String,
    text: String,
}

const SNIPPETS_HEADER: &str = "\
# Сокращения: набрали trigger и пробел (Enter, Tab) — получили text.
# Сокращение срабатывает в любой раскладке. В тексте можно использовать:
#   {date} — сегодняшняя дата, {time} — время, {clipboard} — содержимое буфера обмена,
#   {cursor} — где оставить курсор после вставки.
#
# [[snippet]]
# trigger = \"адр\"
# text = \"г. Москва, ул. Тверская, д. 1\"

";

pub fn load_snippets() -> Vec<Snippet> {
    std::fs::read_to_string(snippets_path()).ok().and_then(|s| parse_snippets(&s)).unwrap_or_default()
}

pub fn parse_snippets(text: &str) -> Option<Vec<Snippet>> {
    let file: SnippetsFile =
        toml::from_str(text).map_err(|e| debug_log!("snippets.toml не читается: {e}")).ok()?;
    Some(
        file.snippets
            .into_iter()
            .filter(|s| !s.trigger.trim().is_empty())
            .map(|s| Snippet { trigger: s.trigger.trim().to_string(), text: s.text })
            .collect(),
    )
}

pub fn snippets_to_string(snippets: &[Snippet]) -> String {
    let file = SnippetsFile {
        snippets: snippets
            .iter()
            .map(|s| SnippetEntry { trigger: s.trigger.clone(), text: s.text.clone() })
            .collect(),
    };
    format!("{SNIPPETS_HEADER}{}", toml::to_string(&file).unwrap_or_default())
}

pub fn write_file(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let c = Config {
            excluded_apps: vec!["keepass.exe".into()],
            mode: ModeName::Careful,
            ..Config::default()
        };
        let back: Config = toml::from_str(&config_to_string(&c)).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn snippets_roundtrip() {
        let s = vec![Snippet {
            trigger: "адр".into(),
            text: "строка 1
строка \"2\""
                .into(),
        }];
        assert_eq!(parse_snippets(&snippets_to_string(&s)).unwrap(), s);
    }

    #[test]
    fn partial_config_uses_defaults() {
        let c: Config = toml::from_str("mode = \"aggressive\"").unwrap();
        assert_eq!(c.mode, ModeName::Aggressive);
        assert!(c.auto && c.careful_in_dev_tools);
    }

    #[test]
    fn learning_roundtrip() {
        let mut l = Learning::default();
        let keys: Vec<engine::Stroke> = "ubuf"
            .chars()
            .map(|c| {
                let (vk, shift) = layouts::locate(c, Lang::En).unwrap();
                engine::Stroke { vk, shift, caps: false }
            })
            .collect();
        l.learn(&keys, Lang::Ru, 2);
        let text = learning_to_string(&l);
        assert!(text.contains("# гига"));
        let file: LearnedFile = toml::from_str(&text).unwrap();
        assert_eq!(file.words["ubuf"].score, 2);
        assert_eq!(file.words["ubuf"].lang, "RU");
    }
}
