//! Значок в трее: рисуется кодом под системный размер, поэтому чёткий при любом масштабе.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
    GdiFlush, SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY,
    DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE, DT_VCENTER, FW_BOLD, TRANSPARENT,
};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, GetSystemMetrics, HICON, ICONINFO, SM_CXSMICON,
};

/// Квадратный значок с текстом (`EN`, `RU`). Цвета — 0xRRGGBB.
pub fn text_icon(text: &str, background: u32, foreground: u32) -> HICON {
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON).max(16);
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size, // сверху вниз
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..zeroed()
        };
        let mut bits: *mut core::ffi::c_void = null_mut();
        let dc = CreateCompatibleDC(null_mut());
        let color = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let old_bmp = SelectObject(dc, color);

        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
        pixels.fill(background & 0xFF_FFFF);

        let font = CreateFontW(
            -(size * 11 / 16),
            0,
            0,
            0,
            FW_BOLD as i32,
            0,
            0,
            0,
            0,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            crate::wide("Segoe UI").as_ptr(),
        );
        let old_font = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        // GDI хранит цвет как 0x00BBGGRR.
        let fg = ((foreground & 0xFF) << 16) | (foreground & 0xFF00) | ((foreground >> 16) & 0xFF);
        SetTextColor(dc, fg);
        let mut rect = RECT { left: 0, top: 0, right: size, bottom: size };
        let mut wtext: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(
            dc,
            wtext.as_mut_ptr(),
            wtext.len() as i32,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        GdiFlush();

        // GDI не умеет в альфу: делаем все пиксели непрозрачными.
        for p in pixels.iter_mut() {
            *p |= 0xFF00_0000;
        }

        SelectObject(dc, old_font);
        SelectObject(dc, old_bmp);
        DeleteObject(font);
        DeleteDC(dc);

        let mask = CreateBitmap(size, size, 1, 1, null());
        let icon_info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&icon_info);
        DeleteObject(mask);
        DeleteObject(color);
        icon
    }
}

/// Иконка в области уведомлений, принадлежащая окну `hwnd`.
pub struct TrayIcon {
    data: NOTIFYICONDATAW,
}

impl TrayIcon {
    /// `callback_msg` — сообщение, которое окно получит при щелчке по значку.
    pub fn new(hwnd: HWND, callback_msg: u32) -> Self {
        let mut data: NOTIFYICONDATAW = unsafe { zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = callback_msg;
        TrayIcon { data }
    }

    /// Показывает значок. Вызывается и при старте, и после перезапуска Проводника.
    pub fn show(&mut self) -> bool {
        unsafe { Shell_NotifyIconW(NIM_ADD, &self.data) != 0 }
    }

    /// Меняет картинку и подсказку. Старая картинка уничтожается.
    pub fn update(&mut self, icon: HICON, tip: &str) {
        let old = self.data.hIcon;
        self.data.hIcon = icon;
        let tip: Vec<u16> = tip.encode_utf16().take(self.data.szTip.len() - 1).collect();
        self.data.szTip = [0; 128];
        self.data.szTip[..tip.len()].copy_from_slice(&tip);
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &self.data);
            if !old.is_null() {
                DestroyIcon(old);
            }
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &self.data);
            if !self.data.hIcon.is_null() {
                DestroyIcon(self.data.hIcon);
            }
        }
    }
}
