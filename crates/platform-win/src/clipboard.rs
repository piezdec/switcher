//! Буфер обмена. Буфер может быть занят другим процессом — открываем с повторами.

use std::time::Duration;
use windows_sys::Win32::Foundation::{GlobalFree, HANDLE};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber,
    OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};

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

/// Содержимое буфера обмена во всех форматах: текст, картинки, скопированные файлы, оформление.
/// Снимается перед переводом выделенного и возвращается после — буфер остаётся как был.
pub struct Snapshot(Vec<(u32, Vec<u8>)>);

/// Форматы, данные которых — не блок памяти (GDI-объекты, «рисует владелец»). Скопировать их
/// нельзя, но у картинок Windows всегда даёт и копируемый вариант (`CF_DIB`).
fn is_handle_format(format: u32) -> bool {
    const CF_BITMAP: u32 = 2;
    const CF_METAFILEPICT: u32 = 3;
    const CF_PALETTE: u32 = 9;
    const CF_ENHMETAFILE: u32 = 14;
    matches!(format, CF_BITMAP | CF_METAFILEPICT | CF_PALETTE | CF_ENHMETAFILE)
        || (0x80..=0x8E).contains(&format) // CF_OWNERDISPLAY, CF_DSP*
        || (0x300..=0x3FF).contains(&format) // CF_GDIOBJFIRST..CF_GDIOBJLAST
}

/// Снимок буфера. `None` — буфер не открылся (занят другой программой).
pub fn snapshot() -> Option<Snapshot> {
    let _open = Open::with_retry()?;
    let mut formats = Vec::new();
    let mut format = 0;
    loop {
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }
        if is_handle_format(format) {
            continue;
        }
        unsafe {
            let handle = GetClipboardData(format);
            if handle.is_null() {
                continue;
            }
            let size = GlobalSize(handle);
            let ptr = GlobalLock(handle) as *const u8;
            if ptr.is_null() {
                continue;
            }
            formats.push((format, std::slice::from_raw_parts(ptr, size).to_vec()));
            GlobalUnlock(handle);
        }
    }
    Some(Snapshot(formats))
}

/// Возвращает снимок в буфер. Пустой снимок — пустой буфер.
pub fn restore(snapshot: &Snapshot) -> bool {
    let Some(_open) = Open::with_retry() else { return false };
    unsafe {
        EmptyClipboard();
        let mut ok = true;
        for (format, data) in &snapshot.0 {
            let mem = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1));
            if mem.is_null() {
                ok = false;
                continue;
            }
            let ptr = GlobalLock(mem) as *mut u8;
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            GlobalUnlock(mem);
            if SetClipboardData(*format, mem as HANDLE).is_null() {
                GlobalFree(mem);
                ok = false;
            }
        }
        ok
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
