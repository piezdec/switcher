//! Горячие клавиши: разбор из строк конфига («Shift+Pause», «Ctrl+Alt+Space», «DoubleShift»)
//! и распознавание в потоке хука.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{Duration, Instant};

/// Двойное нажатие модификатора должно уложиться в этот интервал.
const DOUBLE_TAP: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    /// Обычное сочетание: модификаторы + клавиша.
    Chord { ctrl: bool, alt: bool, shift: bool, win: bool, vk: u16 },
    /// Двойное нажатие модификатора без других клавиш.
    Double(Modifier),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Shift,
    Ctrl,
    Alt,
}

impl Modifier {
    fn of(vk: u16) -> Option<Modifier> {
        match vk {
            0x10 | 0xA0 | 0xA1 => Some(Modifier::Shift),
            0x11 | 0xA2 | 0xA3 => Some(Modifier::Ctrl),
            0x12 | 0xA4 | 0xA5 => Some(Modifier::Alt),
            _ => None,
        }
    }
}

/// Имена клавиш для конфига. Буквы и цифры — сами по себе, F1–F24 — отдельно.
#[rustfmt::skip]
const NAMES: &[(&str, u16)] = &[
    ("Pause", 0x13), ("Space", 0x20), ("Enter", 0x0D), ("Tab", 0x09), ("Esc", 0x1B),
    ("Backspace", 0x08), ("Insert", 0x2D), ("Delete", 0x2E), ("Home", 0x24), ("End", 0x23),
    ("PageUp", 0x21), ("PageDown", 0x22), ("Left", 0x25), ("Up", 0x26), ("Right", 0x27),
    ("Down", 0x28), ("CapsLock", 0x14), ("ScrollLock", 0x91), ("PrintScreen", 0x2C),
    ("Apps", 0x5D), ("`", 0xC0), ("-", 0xBD), ("=", 0xBB), ("[", 0xDB), ("]", 0xDD),
    ("\\", 0xDC), (";", 0xBA), ("'", 0xDE), (",", 0xBC), (".", 0xBE), ("/", 0xBF),
];

pub fn key_name(vk: u16) -> Option<String> {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => Some((vk as u8 as char).to_string()),
        0x70..=0x87 => Some(format!("F{}", vk - 0x6F)),
        _ => NAMES.iter().find(|(_, v)| *v == vk).map(|(n, _)| n.to_string()),
    }
}

fn key_code(name: &str) -> Option<u16> {
    let upper = name.to_ascii_uppercase();
    if upper.len() == 1 {
        let c = upper.as_bytes()[0];
        if c.is_ascii_alphanumeric() {
            return Some(c as u16);
        }
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x6F + n);
        }
    }
    NAMES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| *v)
}

impl Hotkey {
    pub fn parse(s: &str) -> Option<Hotkey> {
        let s = s.trim();
        match s.to_ascii_lowercase().as_str() {
            "" => return None,
            "doubleshift" => return Some(Hotkey::Double(Modifier::Shift)),
            "doublectrl" => return Some(Hotkey::Double(Modifier::Ctrl)),
            "doublealt" => return Some(Hotkey::Double(Modifier::Alt)),
            _ => {}
        }
        let (mut ctrl, mut alt, mut shift, mut win) = (false, false, false, false);
        let mut vk = None;
        // «+» как отдельная клавиша не поддерживается — она же разделитель.
        for part in s.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" => alt = true,
                "shift" => shift = true,
                "win" => win = true,
                _ => vk = Some(key_code(part)?),
            }
        }
        Some(Hotkey::Chord { ctrl, alt, shift, win, vk: vk? })
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Hotkey::Double(m) => write!(f, "Double{m:?}"),
            Hotkey::Chord { ctrl, alt, shift, win, vk } => {
                for (on, name) in [(ctrl, "Ctrl+"), (alt, "Alt+"), (shift, "Shift+"), (win, "Win+")] {
                    if on {
                        f.write_str(name)?;
                    }
                }
                f.write_str(&key_name(vk).unwrap_or_else(|| format!("0x{vk:02X}")))
            }
        }
    }
}

/// Назначения в конфиге. Пустая строка — не назначено.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// Перевести текущее или последнее слово (и выучить).
    pub convert_word: String,
    /// Перевести выделенный текст.
    pub convert_selection: String,
    /// Транслитерировать выделенный текст: «Привет» ⇄ «Privet».
    pub transliterate_selection: String,
    /// Включить или выключить автопереключение.
    pub toggle_auto: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        HotkeyConfig {
            convert_word: "Pause".into(),
            convert_selection: "Shift+Pause".into(),
            transliterate_selection: String::new(),
            toggle_auto: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    ConvertWord,
    ConvertSelection,
    TransliterateSelection,
    ToggleAuto,
}

/// Что делать с нажатием.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// Не горячая клавиша.
    None,
    /// Горячая клавиша: выполнить команду; `swallow` — не пускать нажатие дальше.
    Command { command: Command, swallow: bool },
    /// Отпускание клавиши, нажатие которой мы проглотили, — тоже глотаем.
    SwallowUp,
}

/// Распознавание горячих клавиш по потоку событий хука.
#[derive(Default)]
pub struct Matcher {
    bindings: Vec<(Hotkey, Command)>,
    /// Клавиша, нажатие которой мы проглотили: её отпускание тоже надо проглотить.
    swallowed: Option<u16>,
    /// Модификатор, нажатый «чисто» (без других клавиш), и когда отпущен в прошлый раз.
    tap_down: Option<Modifier>,
    last_tap: Option<(Modifier, Instant)>,
}

impl Matcher {
    pub fn new(cfg: &HotkeyConfig) -> Matcher {
        let mut m = Matcher::default();
        m.configure(cfg);
        m
    }

    pub fn configure(&mut self, cfg: &HotkeyConfig) {
        self.bindings = [
            (&cfg.convert_word, Command::ConvertWord),
            (&cfg.convert_selection, Command::ConvertSelection),
            (&cfg.transliterate_selection, Command::TransliterateSelection),
            (&cfg.toggle_auto, Command::ToggleAuto),
        ]
        .into_iter()
        .filter_map(|(s, c)| Hotkey::parse(s).map(|h| (h, c)))
        .collect();
    }

    /// Одно событие клавиатуры с состоянием модификаторов на момент события.
    #[allow(clippy::too_many_arguments)]
    pub fn on_event(&mut self, vk: u16, up: bool, ctrl: bool, alt: bool, shift: bool, win: bool) -> Hit {
        if up {
            if self.swallowed == Some(vk) {
                self.swallowed = None;
                return Hit::SwallowUp;
            }
            return self.on_modifier_up(vk);
        }

        if let Some(m) = Modifier::of(vk) {
            // Автоповтор зажатого модификатора не сбивает «чистое» нажатие.
            if self.tap_down != Some(m) {
                self.tap_down = Some(m);
            }
            return Hit::None;
        }
        // Любая другая клавиша — двойного нажатия уже не будет.
        self.tap_down = None;
        self.last_tap = None;

        let pressed = Hotkey::Chord { ctrl, alt, shift, win, vk };
        match self.bindings.iter().find(|(h, _)| *h == pressed) {
            Some(&(_, command)) => {
                self.swallowed = Some(vk);
                Hit::Command { command, swallow: true }
            }
            None => Hit::None,
        }
    }

    fn on_modifier_up(&mut self, vk: u16) -> Hit {
        let Some(m) = Modifier::of(vk) else { return Hit::None };
        if self.tap_down.take() != Some(m) {
            return Hit::None;
        }
        let now = Instant::now();
        let double = matches!(self.last_tap, Some((prev, at)) if prev == m && now - at <= DOUBLE_TAP);
        if double {
            self.last_tap = None;
            if let Some(&(_, command)) = self.bindings.iter().find(|(h, _)| *h == Hotkey::Double(m)) {
                // Сам модификатор пропускаем: Shift без клавиши ничего не печатает.
                return Hit::Command { command, swallow: false };
            }
        } else {
            self.last_tap = Some((m, now));
        }
        Hit::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display() {
        for s in ["Pause", "Shift+Pause", "Ctrl+Alt+Space", "Win+F12", "Ctrl+`", "DoubleShift"] {
            assert_eq!(Hotkey::parse(s).unwrap().to_string(), s);
        }
        assert_eq!(Hotkey::parse("ctrl + shift + k").unwrap().to_string(), "Ctrl+Shift+K");
        assert!(Hotkey::parse("").is_none());
        assert!(Hotkey::parse("Ctrl+Nonsense").is_none());
        assert!(Hotkey::parse("Ctrl+Shift").is_none());
    }

    #[test]
    fn chord_is_swallowed_with_its_release() {
        let mut m = Matcher::new(&HotkeyConfig::default());
        assert_eq!(
            m.on_event(0x13, false, false, false, true, false),
            Hit::Command { command: Command::ConvertSelection, swallow: true }
        );
        assert_eq!(m.on_event(0x13, true, false, false, true, false), Hit::SwallowUp);
        // Pause без Shift — другая команда.
        assert_eq!(
            m.on_event(0x13, false, false, false, false, false),
            Hit::Command { command: Command::ConvertWord, swallow: true }
        );
    }

    #[test]
    fn double_shift() {
        let cfg = HotkeyConfig { convert_word: "DoubleShift".into(), ..HotkeyConfig::default() };
        let mut m = Matcher::new(&cfg);
        let tap = |m: &mut Matcher| {
            m.on_event(0xA0, false, false, false, true, false);
            m.on_event(0xA0, true, false, false, false, false)
        };
        assert_eq!(tap(&mut m), Hit::None);
        assert_eq!(tap(&mut m), Hit::Command { command: Command::ConvertWord, swallow: false });
        // Shift, буква, Shift — это не двойное нажатие.
        tap(&mut m);
        m.on_event(0x41, false, false, false, false, false);
        assert_eq!(tap(&mut m), Hit::None);
    }
}
