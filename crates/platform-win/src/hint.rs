//! Плашка с названием раскладки у текстового курсора: появляется, держится и плавно гаснет.
//! Окно полупрозрачное, поверх всех, не забирает фокус и пропускает щелчки мыши насквозь.
//! Та же плашка, только тёмная и подольше, — предупреждение над значком в трее.

use crate::wide;
use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, DrawTextW,
    EndPaint, FillRect, GetDC, InvalidateRect, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
    SetWindowRgn, CLEARTYPE_QUALITY, DT_CALCRECT, DT_CENTER, DT_SINGLELINE, DT_VCENTER, FW_SEMIBOLD,
    PAINTSTRUCT, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetCursorPos, GetForegroundWindow, GetGUIThreadInfo,
    GetWindowThreadProcessId, KillTimer, RegisterClassW, SetLayeredWindowAttributes, SetTimer, SetWindowPos,
    ShowWindow, GUITHREADINFO, HWND_TOPMOST, LWA_ALPHA, SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE, WM_PAINT,
    WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    WS_POPUP,
};

const TIMER_ID: usize = 1;
const TICK_MS: u32 = 30;
const HOLD: Duration = Duration::from_millis(700);
/// Предупреждение держится дольше: его надо успеть прочитать.
const WARN_HOLD: Duration = Duration::from_millis(1600);
const FADE: Duration = Duration::from_millis(450);
const MAX_ALPHA: f32 = 235.0;

/// Цвета: в момент появления светло-жёлтая, затем оранжевая (0xRRGGBB).
const FLASH: u32 = 0xFF_E0_80;
const ORANGE: u32 = 0xF2_8C_28;
const TEXT: u32 = 0x00_00_00;
/// Предупреждение: тёмное с белым текстом, вспышка чуть светлее.
const WARN_FLASH: u32 = 0x5A_5A_5A;
const WARN_BG: u32 = 0x2B_2B_2B;
/// Рамка предупреждения — чтобы не сливалось с тёмной панелью задач.
const WARN_BORDER: u32 = 0xF2_8C_28;
const WARN_TEXT: u32 = 0xFF_FF_FF;

struct State {
    text: String,
    shown: Option<Instant>,
    /// Предупреждение (тёмное, держится дольше), а не раскладка.
    warn: bool,
}

impl State {
    fn hold(&self) -> Duration {
        if self.warn {
            WARN_HOLD
        } else {
            HOLD
        }
    }
}

thread_local! {
    static STATE: RefCell<State> =
        const { RefCell::new(State { text: String::new(), shown: None, warn: false }) };
}

pub struct Hint {
    hwnd: HWND,
}

fn colorref(rgb: u32) -> COLORREF {
    ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

fn scale(px: i32) -> i32 {
    px * unsafe { GetDpiForSystem() } as i32 / 96
}

impl Hint {
    /// Создаёт скрытое окно плашки в текущем потоке (у потока должен быть цикл сообщений).
    pub fn create() -> Option<Hint> {
        unsafe {
            let instance = GetModuleHandleW(null());
            let class = wide("SwitcherHint");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                class.as_ptr(),
                null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            (!hwnd.is_null()).then_some(Hint { hwnd })
        }
    }

    /// Окно плашки (для отладки и снимков).
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Показывает плашку с текстом у текстового курсора активного окна (или у указателя мыши).
    pub fn show(&self, text: &str) {
        let (w, h) = unsafe { measure(self.hwnd, text) };
        let (x, y) = unsafe { anchor() };
        self.show_at(text, false, (x, y, w, h));
    }

    /// Тёмное предупреждение над прямоугольником `above` (значком в трее), в пределах экрана.
    pub fn warn_above(&self, text: &str, above: RECT) {
        use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST};
        let (w, h) = unsafe { measure(self.hwnd, text) };
        let work = unsafe {
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(MonitorFromRect(&above, MONITOR_DEFAULTTONEAREST), &mut mi);
            mi.rcWork
        };
        let gap = scale(8);
        let x = ((above.left + above.right) / 2 - w / 2).clamp(work.left + gap, (work.right - w - gap).max(work.left));
        let y = (above.top - h - gap).clamp(work.top + gap, (work.bottom - h - gap).max(work.top));
        self.show_at(text, true, (x, y, w, h));
    }

    fn show_at(&self, text: &str, warn: bool, (x, y, w, h): (i32, i32, i32, i32)) {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.text = text.to_string();
            s.shown = Some(Instant::now());
            s.warn = warn;
        });
        unsafe {
            SetWindowPos(self.hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE);
            let radius = scale(8);
            SetWindowRgn(self.hwnd, CreateRoundRectRgn(0, 0, w + 1, h + 1, radius, radius), 1);
            SetLayeredWindowAttributes(self.hwnd, 0, MAX_ALPHA as u8, LWA_ALPHA);
            InvalidateRect(self.hwnd, null(), 1);
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            SetTimer(self.hwnd, TIMER_ID, TICK_MS, None);
        }
    }
}

fn font() -> windows_sys::Win32::Graphics::Gdi::HFONT {
    unsafe {
        CreateFontW(
            -scale(15),
            0,
            0,
            0,
            FW_SEMIBOLD as i32,
            0,
            0,
            0,
            0,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        )
    }
}

/// Размер плашки под текст.
unsafe fn measure(hwnd: HWND, text: &str) -> (i32, i32) {
    let dc = GetDC(hwnd);
    let f = font();
    let old = SelectObject(dc, f);
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let mut w: Vec<u16> = text.encode_utf16().collect();
    DrawTextW(dc, w.as_mut_ptr(), w.len() as i32, &mut r, DT_CALCRECT | DT_SINGLELINE);
    SelectObject(dc, old);
    DeleteObject(f);
    ReleaseDC(hwnd, dc);
    (r.right - r.left + scale(18), r.bottom - r.top + scale(8))
}

/// Куда ставить плашку: под системный текстовый курсор, а если его нет — у указателя мыши.
unsafe fn anchor() -> (i32, i32) {
    let fg = GetForegroundWindow();
    let thread = GetWindowThreadProcessId(fg, null_mut());
    let mut info: GUITHREADINFO = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
    if GetGUIThreadInfo(thread, &mut info) != 0 && !info.hwndCaret.is_null() {
        let mut p = POINT { x: info.rcCaret.left, y: info.rcCaret.bottom };
        ClientToScreen(info.hwndCaret, &mut p);
        return (p.x, p.y + scale(4));
    }
    let mut p = POINT { x: 0, y: 0 };
    GetCursorPos(&mut p);
    (p.x + scale(16), p.y + scale(20))
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let (text, age, warn) = STATE.with(|s| {
                let s = s.borrow();
                (s.text.clone(), s.shown.map_or(s.hold(), |t| t.elapsed()), s.warn)
            });
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let mut rect = ps.rcPaint;
            // Первые 150 мс — вспышка, потом основной цвет.
            let flash = age < Duration::from_millis(150);
            let bg = match (warn, flash) {
                (false, true) => FLASH,
                (false, false) => ORANGE,
                (true, true) => WARN_FLASH,
                (true, false) => WARN_BG,
            };
            let brush = CreateSolidBrush(colorref(bg));
            FillRect(dc, &rect, brush);
            DeleteObject(brush);
            if warn {
                use windows_sys::Win32::Graphics::Gdi::FrameRgn;
                let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);
                let radius = scale(8);
                let rgn = CreateRoundRectRgn(0, 0, client.right, client.bottom, radius, radius);
                let border = CreateSolidBrush(colorref(WARN_BORDER));
                let width = scale(1).max(1);
                FrameRgn(dc, rgn, border, width, width);
                DeleteObject(border);
                DeleteObject(rgn);
            }
            let f = font();
            let old = SelectObject(dc, f);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, colorref(if warn { WARN_TEXT } else { TEXT }));
            let mut w: Vec<u16> = text.encode_utf16().collect();
            windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect);
            DrawTextW(dc, w.as_mut_ptr(), w.len() as i32, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            SelectObject(dc, old);
            DeleteObject(f);
            EndPaint(hwnd, &ps);
            0
        }
        WM_TIMER if wparam == TIMER_ID => {
            let (age, hold) = STATE.with(|s| {
                let s = s.borrow();
                (s.shown.map_or(s.hold() + FADE, |t| t.elapsed()), s.hold())
            });
            if age >= hold + FADE {
                KillTimer(hwnd, TIMER_ID);
                ShowWindow(hwnd, SW_HIDE);
            } else {
                if age > Duration::from_millis(150) && age < Duration::from_millis(150 + TICK_MS as u64 * 2) {
                    InvalidateRect(hwnd, null(), 1); // сменить вспышку на основной цвет
                }
                let alpha = if age <= hold {
                    MAX_ALPHA
                } else {
                    MAX_ALPHA * (1.0 - (age - hold).as_secs_f32() / FADE.as_secs_f32())
                };
                SetLayeredWindowAttributes(hwnd, 0, alpha.max(0.0) as u8, LWA_ALPHA);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
