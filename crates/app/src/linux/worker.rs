//! Рабочий поток Linux: исправления, повтор придержанных нажатий, выделенный текст, файлы.
//!
//! Текст печатается физическими клавишами через виртуальную клавиатуру — после того, как
//! раскладка переключена на нужный язык. Символов, которых нет на клавиатуре, так не напечатать;
//! для них пробуем `wtype` (Hyprland, Sway).

use crate::config;
use crate::controller::{Action, SelectionMode, Shared};
use engine::{translit, Fix, Lang, Tail};
use platform_linux::desktop::Desktop;
use platform_linux::input::{Output, RawKeyEvent};
use platform_linux::{keymap, misc};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

pub struct Worker {
    pub desktop: Arc<Desktop>,
    pub out: Output,
    pub caps: Arc<AtomicBool>,
}

pub fn spawn(shared: Arc<Shared>, w: Worker) -> JoinHandle<()> {
    thread::Builder::new()
        .name("switcher-worker".into())
        .spawn(move || run(&shared, &w))
        .expect("не удалось запустить рабочий поток")
}

fn run(shared: &Shared, w: &Worker) {
    loop {
        let action = {
            let mut q = shared.lock();
            loop {
                if let Some(a) = q.actions.pop_front() {
                    break a;
                }
                q.pending = false;
                q = shared.wake.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        match action {
            Action::Quit => return,
            Action::Fix(fix) => w.apply_fix(fix),
            Action::Replay(ev) => {
                let value = if ev.up {
                    0
                } else if ev.raw.repeat {
                    2
                } else {
                    1
                };
                w.out.forward(RawKeyEvent { code: ev.raw.code, value });
            }
            Action::Selection(mode) => w.convert_selection(mode),
            Action::WriteFile { path, contents } => {
                if let Err(e) = config::write_file(&path, &contents) {
                    say!("ОШИБКА: не удалось записать {}: {e}", path.display());
                }
            }
        }
    }
}

impl Worker {
    fn apply_fix(&self, fix: Fix) {
        if let Some(lang) = fix.switch_to {
            if !self.desktop.switch(lang) {
                say!("ОШИБКА: не удалось переключить раскладку на {lang}");
            }
        }
        self.out.tap(keymap::KEY_BACKSPACE, fix.erase);
        let (mut text, mut cursor_back) = if fix.expand_macros {
            crate::macros::expand(&fix.text, now(), misc::clipboard_text)
        } else {
            (fix.text, 0)
        };
        let tail_key = match fix.tail {
            Tail::None => None,
            Tail::Char(c) => {
                text.push(c);
                None
            }
            Tail::Key(vk) => keymap::code_of(vk),
        };
        if tail_key.is_some() {
            cursor_back = 0;
        } else if cursor_back > 0 && matches!(fix.tail, Tail::Char(_)) {
            cursor_back += 1;
        }
        self.type_text(&text);
        if let Some(code) = tail_key {
            self.out.tap(code, 1);
        }
        if cursor_back > 0 {
            self.out.tap(keymap::KEY_LEFT, cursor_back);
        }
        if fix.caps_lock_off && self.caps.load(Ordering::Relaxed) {
            self.out.tap(keymap::KEY_CAPSLOCK, 1);
            self.caps.store(false, Ordering::Relaxed);
        }
    }

    /// Печать физическими клавишами. Символ ищется в текущей раскладке, затем в другой
    /// (с временным переключением); в конце возвращаем исходную раскладку.
    fn type_text(&self, text: &str) {
        let state = self.desktop.state();
        let start = state.lang();
        let mut current = start;
        for c in text.chars() {
            match c {
                '\r' => continue,
                '\n' => {
                    self.out.tap(keymap::KEY_ENTER, 1);
                    continue;
                }
                _ => {}
            }
            let candidates = current.into_iter().chain(current.map(Lang::other)).chain(Lang::ALL);
            let found = candidates.into_iter().find_map(|lang| {
                let (vk, shift) = layouts::locate(c, lang)?;
                Some((lang, keymap::code_of(vk)?, shift))
            });
            match found {
                Some((lang, code, shift)) => {
                    if current != Some(lang) && self.desktop.switch(lang) {
                        current = Some(lang);
                    }
                    // С включённым Caps Lock буквы инвертируют регистр — компенсируем Shift.
                    let caps = self.caps.load(Ordering::Relaxed) && c.is_alphabetic();
                    self.out.tap_with_shift(code, shift ^ caps);
                }
                None => {
                    if !misc::wtype(&c.to_string()) {
                        eprintln!("switcher: символ «{c}» не напечатать (нет на клавиатуре и нет wtype)");
                        diag!("ОШИБКА: символ вне клавиатуры не напечатан (нет wtype)");
                    }
                }
            }
        }
        if let (Some(s), Some(c)) = (start, current) {
            if s != c {
                self.desktop.switch(s);
            }
        }
    }

    /// Выделенный текст берём из primary selection — буфер обмена не трогаем.
    fn convert_selection(&self, mode: SelectionMode) {
        let Some(text) = misc::selected_text().filter(|t| !t.is_empty()) else {
            say!("выделения нет (или не установлен wl-clipboard)");
            return;
        };
        let cyrillic = text.chars().filter(|c| matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё')).count();
        let latin = text.chars().filter(char::is_ascii_alphabetic).count();
        let converted = match mode {
            SelectionMode::Layout => {
                let (from, to) = if cyrillic > latin { (Lang::Ru, Lang::En) } else { (Lang::En, Lang::Ru) };
                self.desktop.switch(to);
                layouts::convert_str(&text, from, to)
            }
            SelectionMode::Translit(style) => translit::transliterate(&text, style),
        };
        self.type_text(&converted);
    }
}

/// Текущие дата и время из `date` (без лишних зависимостей).
fn now() -> crate::macros::Now {
    let out = Command::new("date").arg("+%d %m %Y %H %M").output().ok();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let v: Vec<u32> = text.split_whitespace().filter_map(|s| s.parse().ok()).collect();
    match v.as_slice() {
        [d, m, y, h, min] => (*d, *m, *y, *h, *min),
        _ => (1, 1, 1970, 0, 0),
    }
}
