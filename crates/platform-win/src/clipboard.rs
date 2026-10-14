//! Текст в буфере обмена. Буфер может быть занят другим процессом — открываем с повторами.

use std::time::Duration;
use windows_sys::Win32::Foundation::{GlobalFree, HANDLE};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

const CF_UNICODETEXT: u32 = 13;

/// Открытый буфер обмена; закрывается при уничтожении.
struct Open;

impl Open {
    fn with_retry() -> Option<Open> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Some(Open);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

/// Счётчик изменений буфера: по нему видно, что копирование сработало.
pub fn sequence_number() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

pub fn get_text() -> Option<String> {
    let _open = Open::with_retry()?;
    unsafe {
        let handle = GetClipboardData(CF_UNICODETEXT);
        if handle.is_null() {
            return None;
        }
        let ptr = GlobalLock(handle) as *const u16;
        if ptr.is_null() {
            return None;
        }
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
        GlobalUnlock(handle);
        Some(text)
    }
}

/// Кладёт текст в буфер; `None` — очищает буфер.
pub fn set_text(text: Option<&str>) -> bool {
    let Some(_open) = Open::with_retry() else { return false };
    unsafe {
        EmptyClipboard();
        let Some(text) = text else { return true };
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if mem.is_null() {
            return false;
        }
        let ptr = GlobalLock(mem) as *mut u16;
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        GlobalUnlock(mem);
        if SetClipboardData(CF_UNICODETEXT, mem as HANDLE).is_null() {
            GlobalFree(mem);
            return false;
        }
        true
    }
}
