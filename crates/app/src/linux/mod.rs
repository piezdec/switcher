//! Linux: evdev и uinput, Hyprland и KDE Plasma, значок через StatusNotifierItem.

/// Сообщение и в терминал, и в журнал работы.
macro_rules! say {
    ($($t:tt)*) => {{
        let message = format!($($t)*);
        eprintln!("switcher: {message}");
        crate::diag::write(&message);
    }};
}

mod sys;
mod worker;

use crate::controller::{Controller, Shared};
use crate::sys::{KeyEvent, RawKey, Verdict};
use crate::{config, settings};
use engine::Lang;
use platform_linux::desktop::Desktop;
use platform_linux::input::{self, Grabbed, Mouse, Output, RawKeyEvent};
use platform_linux::tray::{TrayCommand, TrayHandle, TrayView};
use platform_linux::{keymap, misc};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixListener};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

const PERMISSIONS_HELP: &str = "\
Нет доступа к клавиатуре. Switcher читает её напрямую (так требует Wayland), для этого нужно:

  sudo usermod -aG input $USER
  echo 'KERNEL==\"uinput\", GROUP=\"input\", MODE=\"0660\", OPTIONS+=\"static_node=uinput\"' \\
    | sudo tee /etc/udev/rules.d/80-switcher-uinput.rules
  echo uinput | sudo tee /etc/modules-load.d/uinput.conf

и перезайдите в систему (или перезагрузитесь).";

/// Одна копия на пользователя: занятое абстрактное имя сокета освобождается само при выходе.
fn single_instance(name: &str) -> Option<UnixListener> {
    let addr = SocketAddr::from_abstract_name(format!("switcher-{name}-{}", user_id())).ok()?;
    UnixListener::bind_addr(&addr).ok()
}

fn user_id() -> String {
    std::env::var("USER").unwrap_or_default()
}

enum Msg {
    Key(RawKeyEvent),
    Click,
    Tray(TrayCommand),
}

pub fn run(after_update: bool) {
    // После обновления прежняя копия ещё завершается — ждём её до 15 секунд.
    let tries = if after_update { 150 } else { 1 };
    let instance = (0..tries).find_map(|i| {
        if i > 0 {
            std::thread::sleep(Duration::from_millis(100));
        }
        single_instance("main")
    });
    let Some(listener) = instance else {
        say!("уже запущен");
        return;
    };
    crate::diag::init(if after_update {
        "основная программа, после обновления"
    } else {
        "основная программа"
    });
    // Прежняя копия уже вышла (мы дождались её) — её файл *.old можно удалить.
    crate::update::cleanup();

    let desktop = Arc::new(Desktop::detect());
    if !desktop.supported() {
        say!(
            "окружение не поддерживается (нужны Hyprland или KDE Plasma). \n             Автопереключение работать не будет: раскладку не узнать и не сменить."
        );
    }
    let missing = desktop.missing();
    if !missing.is_empty() {
        let names: Vec<_> = missing.iter().map(|l| l.code()).collect();
        say!("не найдена раскладка {} — добавьте её в настройках окружения", names.join(", "));
    }

    let output = match Output::create() {
        Ok(o) => o,
        Err(e) => {
            say!("не удалось создать виртуальную клавиатуру ({e}).\n\n{PERMISSIONS_HELP}");
            return;
        }
    };
    let mut keyboards = Vec::new();
    for (path, dev) in input::keyboards() {
        match Grabbed::grab(dev) {
            Ok(g) => keyboards.push(g),
            Err(e) => say!("не удалось захватить {} ({e})", path.display()),
        }
    }
    if keyboards.is_empty() {
        say!("не найдено ни одной клавиатуры.\n\n{PERMISSIONS_HELP}");
        return;
    }

    crate::update::spawn_auto();

    // Модели грузятся заранее: при первом нажатии на это нет времени.
    engine::warm_up();

    let caps = Arc::new(AtomicBool::new(keyboards.iter().any(Grabbed::caps)));
    let (tx, rx) = mpsc::channel::<Msg>();
    // Окно настроек может попросить закрыться (перед перезапуском в новую версию) — через тот же
    // сокет, что охраняет единственный экземпляр.
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut command = String::new();
                let _ = std::io::Read::read_to_string(&mut std::io::Read::take(stream, 16), &mut command);
                if command.trim() == "quit" && tx.send(Msg::Tray(TrayCommand::Quit)).is_err() {
                    return;
                }
            }
        });
    }

    for mut kb in keyboards {
        let tx = tx.clone();
        let caps = caps.clone();
        std::thread::spawn(move || {
            while let Ok(events) = kb.read() {
                caps.store(kb.caps(), Ordering::Relaxed);
                for e in events {
                    if tx.send(Msg::Key(e)).is_err() {
                        return;
                    }
                }
            }
        });
    }
    for (_, dev) in input::mice() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut mouse = Mouse::new(dev);
            while mouse.wait_click().is_ok() {
                if tx.send(Msg::Click).is_err() {
                    return;
                }
            }
        });
    }

    let shared = Arc::new(Shared::default());
    let worker = worker::spawn(
        shared.clone(),
        worker::Worker { desktop: desktop.clone(), out: output.clone(), caps: caps.clone() },
    );
    let mut controller = Controller::new(
        Box::new(sys::LinuxSys { state: desktop.state(), caps: caps.clone() }),
        shared.clone(),
        {
            let config = config::load_config();
            crate::diag::log_start(&config);
            config
        },
        config::load_learning(),
        config::load_snippets(),
    );

    let (tray_tx, tray_rx) = mpsc::channel::<TrayCommand>();
    let mut view = TrayView::default();
    let tray = TrayHandle::start(view.clone(), tray_tx);
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for c in tray_rx {
                if tx.send(Msg::Tray(c)).is_err() {
                    return;
                }
            }
        });
    }

    say!("работает ({})", desktop.name());
    loop {
        match rx.recv_timeout(Duration::from_millis(150)) {
            Ok(Msg::Key(raw)) => {
                let ev = KeyEvent {
                    vk: keymap::vk_of(raw.code),
                    up: raw.value == 0,
                    raw: RawKey { code: raw.code, extended: false, repeat: raw.value == 2 },
                };
                if controller.on_key(&ev) == Verdict::Pass {
                    output.forward(raw);
                }
                continue;
            }
            Ok(Msg::Click) => controller.on_click(),
            Ok(Msg::Tray(TrayCommand::Quit)) => break,
            Ok(Msg::Tray(TrayCommand::RestartToUpdate)) => {
                if crate::update::launch_updated() {
                    break;
                }
            }
            Ok(Msg::Tray(cmd)) => handle_tray(&mut controller, cmd),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let lang = controller.on_timer();
        // Новая версия готова, а вы давно не печатаете — тихо перезапускаемся в неё.
        if crate::update::should_restart_now() && crate::update::launch_updated() {
            break;
        }
        let next = tray_view(&controller, lang, &desktop);
        if next != view {
            view = next;
            tray.update(view.clone());
        }
    }

    shared.quit();
    let _ = worker.join();
    diag!("выход");
}

fn handle_tray(c: &mut Controller, cmd: TrayCommand) {
    match cmd {
        TrayCommand::ToggleAuto => c.update_config(|cfg| cfg.auto = !cfg.auto),
        TrayCommand::ToggleExcludeApp => {
            if let Some(app) = c.last_app().map(str::to_string) {
                c.update_config(|cfg| {
                    if let Some(i) = cfg.excluded_apps.iter().position(|a| *a == app) {
                        cfg.excluded_apps.remove(i);
                    } else {
                        cfg.excluded_apps.push(app);
                    }
                });
            }
        }
        TrayCommand::ToggleAutostart => {
            misc::set_autostart(!misc::autostart_enabled());
        }
        TrayCommand::OpenSettings => open_settings("general"),
        TrayCommand::OpenDictionary => open_settings("dictionary"),
        TrayCommand::OpenLog => misc::open_folder(&crate::diag::path()),
        TrayCommand::OpenDataDir => {
            let dir = config::data_dir();
            let _ = std::fs::create_dir_all(&dir);
            misc::open_folder(&dir);
        }
        TrayCommand::Quit | TrayCommand::RestartToUpdate => {}
    }
}

fn tray_view(c: &Controller, lang: Option<Lang>, desktop: &Desktop) -> TrayView {
    let cfg = c.config();
    let label = lang.map_or("--", Lang::code).to_string();
    let hints = [
        (&cfg.hotkeys.convert_word, "перевести последнее слово"),
        (&cfg.hotkeys.convert_selection, "перевести выделенное"),
        (&cfg.hotkeys.transliterate_selection, "транслитерировать выделенное"),
        (&cfg.hotkeys.toggle_auto, "включить или выключить"),
    ]
    .into_iter()
    .filter(|(k, _)| !k.is_empty())
    .map(|(k, what)| format!("{k} — {what}"))
    .collect();
    let app = c.last_app().map(str::to_string);
    let app_excluded = app.as_deref().is_some_and(|a| c.is_excluded(a));
    let status = if !cfg.auto {
        "выключено"
    } else if c.active_here() {
        "включено"
    } else {
        "не работает в этом окне"
    };
    TrayView {
        tooltip: format!("{label} · автопереключение {status} · {}", desktop.name()),
        label,
        active: c.active_here() && lang.is_some(),
        auto: cfg.auto,
        hints,
        app,
        app_excluded,
        autostart: misc::autostart_enabled(),
        update: crate::update::ready_tag(),
    }
}

/// Окно настроек — отдельный процесс той же программы.
fn open_settings(tab: &str) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).args(["--settings", tab]).spawn();
    }
}

pub fn run_settings(tab: Option<&str>) {
    let Some(_instance) = single_instance("settings") else {
        say!("настройки уже открыты");
        return;
    };
    crate::diag::init("окно настроек");
    if let Err(e) = settings::run(tab) {
        say!("не удалось открыть настройки: {e}");
    }
}

pub fn autostart_enabled() -> bool {
    misc::autostart_enabled()
}

pub fn set_autostart(on: bool) -> bool {
    misc::set_autostart(on)
}

pub fn autostart_registered() -> bool {
    misc::autostart_enabled()
}

pub fn set_autostart_to(exe: &Path) -> bool {
    misc::set_autostart_for(exe)
}

pub fn open_folder(path: &Path) {
    misc::open_folder(path);
}

/// Просит работающий Switcher закрыться (перед перезапуском в новую версию).
pub fn request_main_exit() {
    use std::io::Write;
    let Ok(addr) = SocketAddr::from_abstract_name(format!("switcher-main-{}", user_id())) else { return };
    if let Ok(mut s) = std::os::unix::net::UnixStream::connect_addr(&addr) {
        let _ = s.write_all(b"quit");
    }
}

/// Опросить клавишу напрямую в Linux нельзя — сочетание в настройках ловится через события окна.
pub fn key_down(_vk: u16) -> Option<bool> {
    None
}
