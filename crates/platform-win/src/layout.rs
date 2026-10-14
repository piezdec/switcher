//! Раскладки: какая сейчас в активном окне и как её сменить.

use layouts::Lang;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, GetKeyboardLayoutList, HKL};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, PostMessageW, WM_INPUTLANGCHANGEREQUEST,
};

fn lang_of(hkl: HKL) -> Option<Lang> {
    let langid = (hkl as usize & 0xFFFF) as u16;
    Lang::from_primary_lang_id(langid & 0x3FF)
}

/// Активное окно (как число, чтобы его можно было хранить и сравнивать в любом потоке).
pub fn foreground_window() -> isize {
    unsafe { GetForegroundWindow() as isize }
}

/// Раскладка потока активного окна. В консолях, Electron и браузерах иногда отстаёт от
/// реальности, поэтому приложение держит ещё и собственное представление о ней.
pub fn foreground_lang() -> Option<Lang> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let thread = GetWindowThreadProcessId(hwnd, std::ptr::null_mut());
        lang_of(GetKeyboardLayout(thread))
    }
}

/// Установленные в системе раскладки для поддерживаемых языков.
#[derive(Clone, Copy, Debug, Default)]
pub struct InstalledLayouts {
    en: Option<isize>,
    ru: Option<isize>,
}

impl InstalledLayouts {
    pub fn detect() -> Self {
        let mut list = [std::ptr::null_mut(); 32];
        let n = unsafe { GetKeyboardLayoutList(list.len() as i32, list.as_mut_ptr()) };
        let mut out = InstalledLayouts::default();
        for &hkl in &list[..n.max(0) as usize] {
            match lang_of(hkl) {
                Some(Lang::En) if out.en.is_none() => out.en = Some(hkl as isize),
                Some(Lang::Ru) if out.ru.is_none() => out.ru = Some(hkl as isize),
                _ => {}
            }
        }
        out
    }

    pub fn get(&self, lang: Lang) -> Option<isize> {
        match lang {
            Lang::En => self.en,
            Lang::Ru => self.ru,
        }
    }

    /// Языки, для которых не нашлось раскладки.
    pub fn missing(&self) -> Vec<Lang> {
        Lang::ALL.into_iter().filter(|&l| self.get(l).is_none()).collect()
    }

    /// Просит активное окно переключиться на `lang`. Сообщение асинхронное, но приходит
    /// в очередь раньше синтезированного следом ввода: GetMessage отдаёт отправленные
    /// сообщения прежде событий ввода.
    pub fn request_switch(&self, lang: Lang) -> bool {
        let Some(hkl) = self.get(lang) else { return false };
        unsafe {
            let hwnd = GetForegroundWindow();
            !hwnd.is_null() && PostMessageW(hwnd, WM_INPUTLANGCHANGEREQUEST, 0, hkl) != 0
        }
    }
}
