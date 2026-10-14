//! Синтез ввода через `SendInput`. Каждое событие помечается [`SIGNATURE`] в `dwExtraInfo`,
//! чтобы наш же хук узнавал его и не принимал за нажатие человека.

use crate::hook::KeyEvent;
use std::mem::size_of;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_BACK, VK_CONTROL, VK_LWIN, VK_MENU,
    VK_RETURN, VK_RWIN, VK_SHIFT,
};

/// Метка синтезированного нами ввода («SWTC»).
pub const SIGNATURE: usize = 0x5357_5443;

fn key(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: SIGNATURE },
        },
    }
}

fn send(inputs: &[INPUT]) -> bool {
    if inputs.is_empty() {
        return true;
    }
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    sent as usize == inputs.len()
}

/// Одним пакетом: `erase` раз Backspace, затем `text` юникодом, затем (опционально)
/// нажатие клавиши `tail_vk`. Приложение обрабатывает пакет за один проход очереди,
/// поэтому замена выглядит мгновенной, без мерцания.
pub fn replace_text(erase: usize, text: &str, tail_vk: Option<u16>) -> bool {
    let mut inputs = Vec::with_capacity(erase * 2 + text.len() * 2 + 2);
    for _ in 0..erase {
        inputs.push(key(VK_BACK, 0, 0));
        inputs.push(key(VK_BACK, 0, KEYEVENTF_KEYUP));
    }
    push_text(&mut inputs, text);
    if let Some(vk) = tail_vk {
        // Только нажатие: отпускание придёт от реальной клавиши.
        inputs.push(key(vk, 0, 0));
    }
    send(&inputs)
}

/// Повторяет перехваченное событие клавиатуры как есть.
pub fn replay(ev: &KeyEvent) -> bool {
    let mut flags = 0;
    if ev.up {
        flags |= KEYEVENTF_KEYUP;
    }
    if ev.extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    send(&[key(ev.vk, ev.scan, flags)])
}

/// Печатает текст юникодом (поверх выделения, если оно есть). Перевод строки — клавишей Enter,
/// `\r` пропускается.
pub fn type_text(text: &str) -> bool {
    let mut inputs = Vec::with_capacity(text.len() * 2);
    push_text(&mut inputs, text);
    send(&inputs)
}

/// Нажимает клавишу `count` раз (например, стрелку влево, чтобы поставить курсор).
pub fn tap(vk: u16, count: usize, extended: bool) -> bool {
    let ext = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
    let inputs: Vec<INPUT> =
        (0..count).flat_map(|_| [key(vk, 0, ext), key(vk, 0, ext | KEYEVENTF_KEYUP)]).collect();
    send(&inputs)
}

fn push_text(inputs: &mut Vec<INPUT>, text: &str) {
    for c in text.chars() {
        match c {
            '\r' => {}
            '\n' => {
                inputs.push(key(VK_RETURN, 0, 0));
                inputs.push(key(VK_RETURN, 0, KEYEVENTF_KEYUP));
            }
            _ => {
                let mut buf = [0u16; 2];
                for &unit in c.encode_utf16(&mut buf).iter() {
                    inputs.push(key(0, unit, KEYEVENTF_UNICODE));
                    inputs.push(key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                }
            }
        }
    }
}

/// Сочетание клавиш: модификаторы вниз, клавиша, всё вверх в обратном порядке.
pub fn chord(modifiers: &[u16], vk: u16, extended: bool) -> bool {
    let ext = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
    let mut inputs: Vec<INPUT> = modifiers.iter().map(|&m| key(m, 0, 0)).collect();
    inputs.push(key(vk, 0, ext));
    inputs.push(key(vk, 0, ext | KEYEVENTF_KEYUP));
    inputs.extend(modifiers.iter().rev().map(|&m| key(m, 0, KEYEVENTF_KEYUP)));
    send(&inputs)
}

/// Зажата ли сейчас физически какая-то из клавиш-модификаторов.
pub fn modifiers_held() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
        .iter()
        .any(|&vk| unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0)
}
