//! Низкоуровневые хуки клавиатуры и мыши (`WH_KEYBOARD_LL`, `WH_MOUSE_LL`).
//!
//! Процедура хука вызывается в потоке, который его установил, пока тот крутит цикл сообщений.
//! Она обязана отвечать быстро: если задержаться, Windows молча выкинет хук из цепочки.

use std::cell::RefCell;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
    LLKHF_EXTENDED, LLKHF_INJECTED, LLKHF_UP, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_MBUTTONDOWN,
    WM_RBUTTONDOWN, WM_XBUTTONDOWN,
};

/// Событие клавиатуры в том виде, в каком его видит хук.
#[derive(Clone, Copy, Debug)]
pub struct KeyEvent {
    pub vk: u16,
    pub scan: u16,
    pub up: bool,
    pub extended: bool,
    pub injected: bool,
    /// `dwExtraInfo` — по нему мы узнаём собственный синтезированный ввод.
    pub extra: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    /// Не пропускать событие дальше по системе.
    Swallow,
}

type KeyHandler = Box<dyn FnMut(&KeyEvent) -> Verdict>;
type ClickHandler = Box<dyn FnMut()>;

thread_local! {
    static KEY_HANDLER: RefCell<Option<KeyHandler>> = const { RefCell::new(None) };
    static CLICK_HANDLER: RefCell<Option<ClickHandler>> = const { RefCell::new(None) };
}

/// Установленные хуки; снимаются при уничтожении.
pub struct Hooks {
    keyboard: HHOOK,
    mouse: HHOOK,
}

impl Hooks {
    /// Ставит оба хука на текущий поток. Поток обязан крутить цикл сообщений.
    pub fn install(
        on_key: impl FnMut(&KeyEvent) -> Verdict + 'static,
        on_click: impl FnMut() + 'static,
    ) -> std::io::Result<Hooks> {
        KEY_HANDLER.with(|h| *h.borrow_mut() = Some(Box::new(on_key)));
        CLICK_HANDLER.with(|h| *h.borrow_mut() = Some(Box::new(on_click)));
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0);
            if keyboard.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
            if mouse.is_null() {
                let err = std::io::Error::last_os_error();
                UnhookWindowsHookEx(keyboard);
                return Err(err);
            }
            Ok(Hooks { keyboard, mouse })
        }
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        unsafe {
            UnhookWindowsHookEx(self.keyboard);
            UnhookWindowsHookEx(self.mouse);
        }
        KEY_HANDLER.with(|h| h.borrow_mut().take());
        CLICK_HANDLER.with(|h| h.borrow_mut().take());
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = &*(lparam as *const KBDLLHOOKSTRUCT);
        let ev = KeyEvent {
            vk: info.vkCode as u16,
            scan: info.scanCode as u16,
            up: info.flags & LLKHF_UP != 0,
            extended: info.flags & LLKHF_EXTENDED != 0,
            injected: info.flags & LLKHF_INJECTED != 0,
            extra: info.dwExtraInfo,
        };
        // try_borrow_mut: если обработчик почему-то уже занят (вложенный вызов), просто пропускаем.
        let verdict = KEY_HANDLER.with(|h| match h.try_borrow_mut() {
            Ok(mut slot) => slot.as_mut().map_or(Verdict::Pass, |f| f(&ev)),
            Err(_) => Verdict::Pass,
        });
        if verdict == Verdict::Swallow {
            return 1;
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let msg = wparam as u32;
        if matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN) {
            CLICK_HANDLER.with(|h| {
                if let Ok(mut slot) = h.try_borrow_mut() {
                    if let Some(f) = slot.as_mut() {
                        f();
                    }
                }
            });
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}
