//! Windows: низкоуровневые хуки, SendInput, трей, UI Automation.

mod sys;
mod ui;
mod worker;

use crate::controller::{Controller, Shared};
use crate::{config, settings};
use platform_win::layout::InstalledLayouts;
use platform_win::password::PasswordWatch;
use platform_win::wide;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

pub use ui::error_box;

/// Не даём запустить вторую копию: два хука будут исправлять одно слово дважды.
struct SingleInstance(HANDLE);

impl SingleInstance {
    fn acquire(name: &str) -> Option<Self> {
        unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, wide(name).as_ptr());
            if handle.is_null() {
                return None;
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle);
                return None;
            }
            Some(SingleInstance(handle))
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Одна копия программы. После обновления прежняя копия ещё завершается — ждём её до 15 секунд.
fn acquire_main(after_update: bool) -> Option<SingleInstance> {
    let tries = if after_update { 150 } else { 1 };
    for _ in 0..tries {
        if let Some(i) = SingleInstance::acquire("Local\\switcher-single-instance") {
            return Some(i);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    None
}

pub fn run(after_update: bool) {
    let Some(_instance) = acquire_main(after_update) else {
        debug_log!("уже запущен");
        return;
    };
    crate::diag::init(if after_update {
        "основная программа, после обновления"
    } else {
        "основная программа"
    });
    // Прежняя копия уже вышла (мы дождались её) — её файл *.old можно удалить.
    crate::update::cleanup();
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

    let layouts = InstalledLayouts::detect();
    let missing = layouts.missing();
    diag!(
        "раскладки: найдены {}",
        if missing.is_empty() {
            "русская и английская".to_string()
        } else {
            format!("не все, нет: {missing:?}")
        }
    );
    if !missing.is_empty() {
        let names: Vec<_> = missing.iter().map(|l| l.code()).collect();
        error_box(&format!(
            "В системе не найдена раскладка: {}.\nДобавьте её в параметрах Windows — без неё переключение не сработает.",
            names.join(", ")
        ));
    }

    crate::update::spawn_auto();

    // Модели грузятся заранее: в процедуре хука на это нет времени.
    engine::warm_up();

    let shared = Arc::new(Shared::default());
    let worker = worker::spawn(shared.clone(), layouts);
    let password = PasswordWatch::start();
    let config = config::load_config();
    crate::diag::log_start(&config);
    let controller = Rc::new(RefCell::new(Controller::new(
        Box::new(sys::WinSys::new(password.clone())),
        shared.clone(),
        config,
        config::load_learning(),
        config::load_snippets(),
    )));

    if let Err(e) = ui::run(controller) {
        diag!("СБОЙ: {e}");
        error_box(&e);
    }

    password.stop();
    shared.quit();
    let _ = worker.join();
    diag!("выход");
}

/// Окно настроек: отдельный процесс, одна копия. Повторный запуск выводит открытое окно вперёд.
pub fn run_settings(tab: Option<&str>) {
    let Some(_instance) = SingleInstance::acquire("Local\\switcher-settings") else {
        ui::focus_window(settings::TITLE);
        return;
    };
    crate::diag::init("окно настроек");
    if let Err(e) = settings::run(tab) {
        diag!("СБОЙ: окно настроек не открылось: {e}");
        error_box(&format!("Не удалось открыть настройки: {e}"));
    }
}

pub fn autostart_enabled() -> bool {
    platform_win::autostart::is_enabled()
}

pub fn set_autostart(on: bool) -> bool {
    platform_win::autostart::set_enabled(on)
}

/// Автозапуск включён (на любой путь) — для переезда обновлённой копии.
pub fn autostart_registered() -> bool {
    platform_win::autostart::is_registered()
}

pub fn set_autostart_to(exe: &Path) -> bool {
    platform_win::autostart::set_enabled_for(exe)
}

pub fn open_folder(path: &Path) {
    ui::open_folder(path);
}

/// Просит работающий Switcher закрыться (перед перезапуском в новую версию).
pub fn request_main_exit() {
    ui::close_main_window();
}

/// Зажата ли клавиша прямо сейчас (для записи сочетания в настройках).
/// `None` — платформа так не умеет, сочетание ловится через события окна.
pub fn key_down(vk: u16) -> Option<bool> {
    Some(unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0)
}
