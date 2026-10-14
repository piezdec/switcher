//! Показывает плашку у курсора и через 300 мс сохраняет снимок её окна (для проверки отрисовки).
//!   cargo run -p platform-win --example hint_demo -- out.png

use platform_win::hint::Hint;
use std::time::{Duration, Instant};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};

fn main() {
    let hint = Hint::create().expect("окно плашки");
    hint.show("RU · выучено");
    let start = Instant::now();
    let mut captured = false;
    while start.elapsed() < Duration::from_millis(1400) {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if !captured && start.elapsed() > Duration::from_millis(300) {
            println!("hwnd={}", hint.hwnd() as isize);
            captured = true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
