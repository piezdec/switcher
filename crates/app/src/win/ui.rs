//! Скрытое главное окно: цикл сообщений (он же нужен хукам), значок в трее и его меню.

use crate::config::{self, Config};
use crate::controller::Controller;
use crate::sys::{KeyEvent, RawKey, Verdict as SysVerdict};
use engine::{Lang, Mode};
use platform_win::autostart;
use platform_win::hint::Hint;
use platform_win::hook::{Hooks, KeyEvent as HookEvent, Verdict};
use platform_win::input;
use platform_win::sound::{self, Sound};
use platform_win::tray::{text_icon, TrayIcon};
use platform_win::wide;
use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::sync::atomic::{AtomicPtr, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW, MessageBoxW, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, SetTimer, ShowWindow, TrackPopupMenu,
    TranslateMessage, MB_ICONERROR, MB_OK, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG,
    SW_RESTORE, SW_SHOWNORMAL, TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_COMMAND,
    WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONDBLCLK, WM_NULL, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};

const WM_TRAY: u32 = WM_APP + 1;
const TIMER_ID: usize = 1;
const TIMER_MS: u32 = 150;

const CMD_AUTO: usize = 1;
const CMD_EXIT: usize = 2;
const CMD_CAREFUL_DEV: usize = 3;
const CMD_EXCLUDE_APP: usize = 4;
const CMD_AUTOSTART: usize = 5;
const CMD_OPEN_DATA: usize = 6;
const CMD_SETTINGS: usize = 7;
const CMD_DICTIONARY: usize = 8;
const CMD_RESTART_UPDATE: usize = 9;
const CMD_OPEN_LOG: usize = 13;
const CMD_MODE_CAREFUL: usize = 10;
const CMD_MODE_NORMAL: usize = 11;
const CMD_MODE_AGGRESSIVE: usize = 12;

const ORANGE: u32 = 0xF2_8C_28;
const GREY: u32 = 0x9A_9A_9A;
const BLACK: u32 = 0x00_00_00;

struct Ui {
    controller: Rc<RefCell<Controller>>,
    tray: TrayIcon,
    /// Что сейчас показано на значке: раскладка, включено ли автопереключение, режим.
    shown: Option<(Option<Lang>, bool, Mode)>,
    taskbar_created: u32,
    /// Плашка с раскладкой у курсора.
    hint: Option<Hint>,
    /// Последняя известная раскладка — чтобы заметить смену.
    last_lang: Option<Lang>,
}

/// Окно, которому контроллер шлёт просьбы проиграть звук (из процедуры хука звук не играем).
static UI_HWND: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(null_mut());

const WM_SOUND: u32 = WM_APP + 2;

/// Просит главное окно проиграть звук. Дёшево: только отправка сообщения своему же потоку.
pub fn post_sound(s: Sound) {
    let hwnd = UI_HWND.load(Ordering::Relaxed);
    if !hwnd.is_null() {
        let code = match s {
            Sound::ToRussian => 0,
            Sound::ToEnglish => 1,
            Sound::Click => 2,
        };
        unsafe { PostMessageW(hwnd, WM_SOUND, code, 0) };
    }
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

/// Выводит вперёд окно с таким заголовком (уже открытые настройки).
pub fn focus_window(title: &str) {
    unsafe {
        let hwnd = FindWindowW(null(), wide(title).as_ptr());
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }
}

pub fn error_box(text: &str) {
    unsafe { MessageBoxW(null_mut(), wide(text).as_ptr(), wide("Switcher").as_ptr(), MB_OK | MB_ICONERROR) };
}

/// Создаёт окно, значок и хуки и крутит цикл сообщений до выхода.
pub fn run(controller: Rc<RefCell<Controller>>) -> Result<(), String> {
    let hwnd = create_window().map_err(|e| format!("Не удалось создать окно: {e}"))?;

    UI_HWND.store(hwnd, Ordering::Relaxed);
    let mut tray = TrayIcon::new(hwnd, WM_TRAY);
    tray.show();
    let taskbar_created = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
    UI.with(|ui| {
        *ui.borrow_mut() = Some(Ui {
            controller: controller.clone(),
            tray,
            shown: None,
            taskbar_created,
            hint: Hint::create(),
            last_lang: None,
        })
    });
    refresh_icon();

    let on_key = {
        let c = controller.clone();
        move |ev: &HookEvent| {
            // Наш же синтезированный ввод пропускаем, не анализируя.
            if ev.extra == input::SIGNATURE {
                return Verdict::Pass;
            }
            let e = KeyEvent {
                vk: ev.vk,
                up: ev.up,
                raw: RawKey { code: ev.scan, extended: ev.extended, repeat: false },
            };
            match c.try_borrow_mut().map(|mut c| c.on_key(&e)) {
                Ok(SysVerdict::Swallow) => Verdict::Swallow,
                _ => Verdict::Pass,
            }
        }
    };
    let on_click = {
        let c = controller.clone();
        move || {
            if let Ok(mut c) = c.try_borrow_mut() {
                c.on_click();
            }
        }
    };
    let hooks =
        Hooks::install(on_key, on_click).map_err(|e| format!("Не удалось поставить хук клавиатуры: {e}"))?;

    unsafe {
        SetTimer(hwnd, TIMER_ID, TIMER_MS, None);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    drop(hooks);
    UI.with(|ui| ui.borrow_mut().take());
    Ok(())
}

fn create_window() -> std::io::Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = wide("SwitcherMainWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Switcher").as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(hwnd)
        }
    }
}

/// Перерисовывает значок, если раскладка или режим изменились.
fn refresh_icon() {
    UI.with(|ui| {
        let Ok(mut guard) = ui.try_borrow_mut() else { return };
        let Some(ui) = guard.as_mut() else { return };
        let Ok(mut c) = ui.controller.try_borrow_mut() else { return };
        let state = (c.on_timer(), c.active_here(), c.effective_mode());
        let auto = c.config().auto;
        let indicator = c.config().indicator;
        let note = c.take_note();
        drop(c);

        // Смена раскладки (нашей или пользователя): плашка у курсора. Звук — только при наших
        // переключениях, его играет контроллер.
        let (lang, active, mode) = state;
        if let (Some(prev), Some(now)) = (ui.last_lang, lang) {
            if prev != now && indicator {
                if let Some(hint) = &ui.hint {
                    hint.show(&match note {
                        Some(n) => format!("{} · {n}", now.code()),
                        None => now.code().to_string(),
                    });
                }
            }
        }
        if lang.is_some() {
            ui.last_lang = lang;
        }

        if ui.shown == Some(state) {
            return;
        }
        ui.shown = Some(state);
        let label = lang.map_or("--", Lang::code);
        let background = if active && lang.is_some() { ORANGE } else { GREY };
        let status = match (auto, active) {
            (false, _) => "выключено",
            (true, false) => "не работает в этом окне",
            (true, true) => "включено",
        };
        let mode = match mode {
            Mode::Careful => "осторожный",
            Mode::Normal => "обычный",
            Mode::Aggressive => "активный",
        };
        let update =
            crate::update::ready_tag().map(|t| format!("\nОбновление {t} готово")).unwrap_or_default();
        let tip = format!("Switcher — {label}\nАвтопереключение {status}\nРежим: {mode}{update}");
        ui.tray.update(text_icon(label, background, BLACK), &tip);
    });
}

/// Выполняет действие над контроллером (если UI жив) и обновляет значок.
fn with_controller<R>(f: impl FnOnce(&mut Controller) -> R) -> Option<R> {
    let r = UI.with(|ui| ui.borrow().as_ref().map(|ui| f(&mut ui.controller.borrow_mut())));
    UI.with(|ui| {
        if let Some(ui) = ui.borrow_mut().as_mut() {
            ui.shown = None;
        }
    });
    refresh_icon();
    r
}

fn toggle_auto() {
    with_controller(|c| c.update_config(|cfg| cfg.auto = !cfg.auto));
}

/// Состояние для меню, снятое заранее: пока меню открыто, контроллер не заимствуем.
struct MenuState {
    config: Config,
    last_app: Option<String>,
    autostart: bool,
}

fn show_menu(hwnd: HWND) {
    let Some(state) = with_controller(|c| MenuState {
        config: c.config().clone(),
        last_app: c.last_app().map(str::to_string),
        autostart: autostart::is_enabled(),
    }) else {
        return;
    };
    let cfg = &state.config;
    let mode: Mode = cfg.mode.into();
    let checked = |on: bool| MF_STRING | if on { MF_CHECKED } else { 0 };
    unsafe {
        let modes = CreatePopupMenu();
        for (cmd, m, label) in [
            (CMD_MODE_CAREFUL, Mode::Careful, "Осторожно — только явные ошибки"),
            (CMD_MODE_NORMAL, Mode::Normal, "Обычно"),
            (CMD_MODE_AGGRESSIVE, Mode::Aggressive, "Активно — иногда лишние срабатывания"),
        ] {
            AppendMenuW(modes, checked(mode == m), cmd, wide(label).as_ptr());
        }

        let menu = CreatePopupMenu();
        AppendMenuW(menu, checked(cfg.auto), CMD_AUTO, wide("Автопереключение").as_ptr());
        AppendMenuW(menu, MF_POPUP, modes as usize, wide("Чувствительность").as_ptr());
        AppendMenuW(
            menu,
            checked(cfg.careful_in_dev_tools),
            CMD_CAREFUL_DEV,
            wide("Осторожно в терминалах и редакторах кода").as_ptr(),
        );
        if let Some(app) = &state.last_app {
            let excluded = cfg.excluded_apps.contains(app);
            let label = format!("Не переключать в {app}");
            AppendMenuW(menu, checked(excluded), CMD_EXCLUDE_APP, wide(&label).as_ptr());
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        for (key, what) in [
            (&cfg.hotkeys.convert_word, "перевести последнее слово"),
            (&cfg.hotkeys.convert_selection, "перевести выделенное"),
            (&cfg.hotkeys.transliterate_selection, "транслитерировать выделенное"),
            (&cfg.hotkeys.toggle_auto, "включить или выключить"),
        ] {
            if !key.is_empty() {
                AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, wide(&format!("{key} — {what}")).as_ptr());
            }
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(
            menu,
            checked(state.autostart),
            CMD_AUTOSTART,
            wide("Запускать вместе с Windows").as_ptr(),
        );
        if let Some(tag) = crate::update::ready_tag() {
            let label = format!("Перезапустить и обновить до {tag}");
            AppendMenuW(menu, MF_STRING, CMD_RESTART_UPDATE, wide(&label).as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
        }
        AppendMenuW(menu, MF_STRING, CMD_DICTIONARY, wide("Выученные слова…").as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_SETTINGS, wide("Настройки…").as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_OPEN_DATA, wide("Открыть папку настроек").as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_OPEN_LOG, wide("Открыть журнал работы").as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_EXIT, wide("Выход").as_ptr());
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Без этого меню не закрывается по щелчку мимо него.
        SetForegroundWindow(hwnd);
        // Пока меню открыто, крутится вложенный цикл сообщений: никаких заимствований UI здесь.
        TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        // Подменю уничтожается вместе с меню.
        DestroyMenu(menu);
    }
}

fn set_mode(mode: Mode) {
    with_controller(|c| c.update_config(|cfg| cfg.mode = mode.into()));
}

fn toggle_excluded_app() {
    with_controller(|c| {
        let Some(app) = c.last_app().map(str::to_string) else { return };
        c.update_config(|cfg| {
            if let Some(i) = cfg.excluded_apps.iter().position(|a| *a == app) {
                cfg.excluded_apps.remove(i);
            } else {
                cfg.excluded_apps.push(app);
            }
        });
    });
}

/// Окно настроек — отдельный процесс того же exe.
fn open_settings(tab: &str) {
    let started = std::env::current_exe()
        .and_then(|exe| std::process::Command::new(exe).args(["--settings", tab]).spawn());
    if let Err(e) = started {
        error_box(&format!("Не удалось открыть настройки: {e}"));
    }
}

fn open_data_dir() {
    let dir = config::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    open_folder(&dir);
}

pub fn open_folder(dir: &std::path::Path) {
    let path = wide(&dir.to_string_lossy());
    unsafe {
        ShellExecuteW(null_mut(), wide("open").as_ptr(), path.as_ptr(), null(), null(), SW_SHOWNORMAL);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_SOUND => {
            let s = match wparam {
                0 => Sound::ToRussian,
                1 => Sound::ToEnglish,
                _ => Sound::Click,
            };
            play_with_override(s);
            0
        }
        WM_TIMER if wparam == TIMER_ID => {
            refresh_icon();
            // Новая версия готова, а вы давно не печатаете — тихо перезапускаемся в неё.
            if crate::update::should_restart_now() && crate::update::launch_updated() {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_TRAY => {
            match (lparam & 0xFFFF) as u32 {
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                WM_LBUTTONDBLCLK => toggle_auto(),
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            match wparam & 0xFFFF {
                CMD_AUTO => toggle_auto(),
                CMD_CAREFUL_DEV => {
                    with_controller(|c| {
                        c.update_config(|cfg| cfg.careful_in_dev_tools = !cfg.careful_in_dev_tools)
                    });
                }
                CMD_MODE_CAREFUL => set_mode(Mode::Careful),
                CMD_MODE_NORMAL => set_mode(Mode::Normal),
                CMD_MODE_AGGRESSIVE => set_mode(Mode::Aggressive),
                CMD_EXCLUDE_APP => toggle_excluded_app(),
                CMD_AUTOSTART => {
                    if !autostart::set_enabled(!autostart::is_enabled()) {
                        error_box("Не удалось изменить автозапуск.");
                    }
                }
                CMD_OPEN_DATA => open_data_dir(),
                CMD_OPEN_LOG => open_folder(&crate::diag::path()),
                CMD_SETTINGS => open_settings("general"),
                CMD_DICTIONARY => open_settings("dictionary"),
                CMD_RESTART_UPDATE => {
                    if crate::update::launch_updated() {
                        DestroyWindow(hwnd);
                    }
                }
                CMD_EXIT => {
                    DestroyWindow(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => {
            // Проводник перезапустился — значок надо добавить заново.
            let recreate = UI.with(|ui| ui.borrow().as_ref().is_some_and(|ui| ui.taskbar_created == msg));
            if recreate {
                UI.with(|ui| {
                    if let Some(ui) = ui.borrow_mut().as_mut() {
                        ui.tray.show();
                        ui.shown = None;
                    }
                });
                refresh_icon();
                return 0;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}

/// Свой звук вместо встроенного: wav-файл в папке настроек. Для переключения сначала
/// ищется файл под направление (`switch_ru.wav`, `switch_en.wav`), потом общий `switch.wav`;
/// для щелчка — `click.wav`.
fn play_with_override(s: Sound) {
    let dir = config::data_dir();
    let candidates: &[&str] = match s {
        Sound::ToRussian => &["switch_ru.wav", "switch.wav"],
        Sound::ToEnglish => &["switch_en.wav", "switch.wav"],
        Sound::Click => &["click.wav"],
    };
    if !candidates.iter().any(|name| sound::play_file(&dir.join(name))) {
        sound::play(s);
    }
}

/// Просит работающий Switcher закрыться (из окна настроек — перед перезапуском в новую версию).
pub fn close_main_window() {
    unsafe {
        let hwnd = FindWindowW(wide("SwitcherMainWindow").as_ptr(), null());
        if !hwnd.is_null() {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
    }
}
