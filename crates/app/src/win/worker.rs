//! Рабочий поток: выполняет исправления, досылает перехваченные нажатия, переводит
//! выделенный текст и пишет файлы. Здесь можно позволить себе медленные и блокирующие
//! вызовы — хук от них не страдает.

use crate::config;
use crate::controller::{Action, SelectionMode, Shared};
use engine::{selection, text, translit, Fix, Tail};
use platform_win::hook::KeyEvent as HookEvent;
use platform_win::layout::InstalledLayouts;
use platform_win::{clipboard, input};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CAPITAL, VK_CONTROL, VK_INSERT, VK_LEFT,
};

pub fn spawn(shared: Arc<Shared>, layouts: InstalledLayouts) -> JoinHandle<()> {
    thread::Builder::new()
        .name("switcher-worker".into())
        .spawn(move || run(&shared, layouts))
        .expect("не удалось запустить рабочий поток")
}

fn run(shared: &Shared, layouts: InstalledLayouts) {
    loop {
        let action = {
            let mut q = shared.lock();
            loop {
                if let Some(a) = q.actions.pop_front() {
                    break a;
                }
                // Очередь пуста — исправление и повтор ввода закончены, хук снова пропускает нажатия.
                q.pending = false;
                q = shared.wake.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        match action {
            Action::Quit => return,
            Action::Fix(fix) => apply_fix(fix, &layouts),
            Action::Replay(ev) => {
                input::replay(&HookEvent {
                    vk: ev.vk,
                    scan: ev.raw.code,
                    up: ev.up,
                    extended: ev.raw.extended,
                    injected: false,
                    extra: 0,
                });
            }
            Action::Selection(mode) => convert_selection(mode, &layouts),
            Action::SwitchLayout(lang) => {
                if !layouts.request_switch(lang) {
                    diag!("ОШИБКА: не удалось переключить раскладку на {lang}");
                }
            }
            Action::WriteFile { path, contents } => {
                if let Err(e) = config::write_file(&path, &contents) {
                    diag!("ОШИБКА: не удалось записать {}: {e}", path.display());
                }
            }
        }
    }
}

fn apply_fix(fix: Fix, layouts: &InstalledLayouts) {
    // Сначала просим сменить раскладку: это сообщение окно обработает раньше
    // синтезированного ввода, и досланные следом клавиши уже попадут в новую раскладку.
    if let Some(lang) = fix.switch_to {
        if !layouts.request_switch(lang) {
            diag!("ОШИБКА: не удалось переключить раскладку на {lang}");
        }
    }
    let (mut text, mut cursor_back) =
        if fix.expand_macros { expand_macros(&fix.text) } else { (fix.text, 0) };
    let tail_vk = match fix.tail {
        Tail::None => None,
        Tail::Char(c) => {
            text.push(c);
            None
        }
        Tail::Key(vk) => Some(vk),
    };
    // Курсор возвращаем, только если после текста не нажимается Enter или Tab.
    if tail_vk.is_some() {
        cursor_back = 0;
    } else if cursor_back > 0 && matches!(fix.tail, Tail::Char(_)) {
        cursor_back += 1;
    }
    if !input::replace_text(fix.erase, &text, tail_vk) {
        diag!("ОШИБКА: Windows не пропустила ввод (SendInput) — окно запущено от администратора?");
    }
    if cursor_back > 0 {
        input::tap(VK_LEFT, cursor_back, true);
    }
    if fix.caps_lock_off && unsafe { GetKeyState(VK_CAPITAL as i32) } & 1 != 0 {
        input::tap(VK_CAPITAL, 1, false);
    }
}

/// Раскрывает макросы сокращения с датой из WinAPI и буфером обмена Windows.
fn expand_macros(text: &str) -> (String, usize) {
    let t = unsafe {
        let mut st = std::mem::zeroed();
        GetLocalTime(&mut st);
        st
    };
    let now = (t.wDay as u32, t.wMonth as u32, t.wYear as u32, t.wHour as u32, t.wMinute as u32);
    crate::macros::expand(text, now, clipboard::get_text)
}

/// Перевод или транслитерация выделенного текста: копируем, преобразуем, печатаем поверх
/// выделения, возвращаем буфер обмена как был (текстовое содержимое).
fn convert_selection(mode: SelectionMode, layouts: &InstalledLayouts) {
    // Ждём, пока отпустят модификаторы горячей клавиши, иначе копирование станет, например,
    // Ctrl+Shift+Insert.
    let deadline = Instant::now() + Duration::from_secs(1);
    while input::modifiers_held() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }

    // Буфер обмена — во всех форматах (картинки, файлы, оформление), чтобы вернуть его как был.
    let saved = clipboard::snapshot();
    let restore = || {
        if let Some(saved) = &saved {
            if !clipboard::restore(saved) {
                diag!("ОШИБКА: не удалось вернуть буфер обмена целиком");
            }
        }
    };
    let before = clipboard::sequence_number();
    // Ctrl+Insert, а не Ctrl+C: в терминале Ctrl+C без выделения прервёт программу.
    input::chord(&[VK_CONTROL], VK_INSERT, true);
    let deadline = Instant::now() + Duration::from_millis(400);
    while clipboard::sequence_number() == before && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    if clipboard::sequence_number() == before {
        diag!("выделения нет — переводить нечего");
        return;
    }
    let Some(text) = clipboard::get_text().filter(|t| !t.is_empty()) else {
        // Выделен не текст (картинка, файл): переводить нечего, буфер возвращаем как был.
        restore();
        return;
    };

    let converted = match mode {
        SelectionMode::Layout => {
            // Переводятся только слова не в той раскладке; раскладку — туда, куда переводили.
            let (converted, switch_to) = selection::convert(&text);
            if let Some(to) = switch_to {
                layouts.request_switch(to);
            }
            converted
        }
        SelectionMode::Translit(style) => translit::transliterate(&text, style),
        SelectionMode::Case => text::cycle_case(&text),
    };

    // Печатаем, а не вставляем: буфер обмена можно вернуть сразу.
    input::type_text(&converted);
    restore();
}
