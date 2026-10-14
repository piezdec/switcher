//! Автозапуск с Windows: значение в `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

use crate::wide;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE: &str = "Switcher";

/// Команда автозапуска для текущего exe.
fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

/// Включён ли автозапуск именно этого exe.
pub fn is_enabled() -> bool {
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(VALUE).as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return false;
    }
    let len = (size as usize / 2).saturating_sub(1);
    let value = String::from_utf16_lossy(&buf[..len]);
    command().is_some_and(|c| c.eq_ignore_ascii_case(&value))
}

pub fn set_enabled(on: bool) -> bool {
    unsafe {
        if on {
            let Some(cmd) = command() else { return false };
            let data = wide(&cmd);
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(VALUE).as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            ) == ERROR_SUCCESS
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(VALUE).as_ptr())
                == ERROR_SUCCESS
        }
    }
}

/// Включает автозапуск конкретного exe (после переезда обновлённой копии в другую папку).
pub fn set_enabled_for(exe: &std::path::Path) -> bool {
    let data = wide(&format!("\"{}\"", exe.display()));
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(VALUE).as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        ) == ERROR_SUCCESS
    }
}

/// Записан ли автозапуск Switcher вообще (на любой путь).
pub fn is_registered() -> bool {
    let mut size = 0u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(VALUE).as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        ) == ERROR_SUCCESS
    }
}
