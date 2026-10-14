//! Раскладка, активное окно и программа — у каждого окружения по-своему.
//!
//! - Hyprland: IPC-сокеты (`.socket.sock` для запросов, `.socket2.sock` для событий).
//! - KDE Plasma: DBus `org.kde.keyboard /Layouts`. Активное окно KDE не сообщает —
//!   профили и исключения программ там пока не работают.
//!
//! Состояние кешируется и обновляется фоновыми потоками по событиям: поток перехвата
//! клавиатуры спрашивает раскладку на каждое нажатие и ждать не должен.

use layouts::Lang;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// Закешированное состояние рабочего стола.
#[derive(Default)]
pub struct DesktopState {
    /// 0 — неизвестно, 1 — английская, 2 — русская.
    lang: AtomicU8,
    window: AtomicU64,
    app: Mutex<Option<String>>,
    fullscreen: AtomicBool,
    pid: AtomicU32,
}

impl DesktopState {
    pub fn lang(&self) -> Option<Lang> {
        match self.lang.load(Ordering::Relaxed) {
            1 => Some(Lang::En),
            2 => Some(Lang::Ru),
            _ => None,
        }
    }

    fn set_lang(&self, lang: Option<Lang>) {
        let v = match lang {
            Some(Lang::En) => 1,
            Some(Lang::Ru) => 2,
            None => 0,
        };
        self.lang.store(v, Ordering::Relaxed);
    }

    /// Активное окно на весь экран (сейчас знает только Hyprland).
    /// PID процесса активного окна (0 — неизвестен; сейчас знает только Hyprland).
    pub fn pid(&self) -> u32 {
        self.pid.load(Ordering::Relaxed)
    }

    pub fn fullscreen(&self) -> bool {
        self.fullscreen.load(Ordering::Relaxed)
    }

    pub fn window(&self) -> u64 {
        self.window.load(Ordering::Relaxed)
    }

    pub fn app(&self) -> Option<String> {
        self.app.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_app(&self, app: Option<String>) {
        *self.app.lock().unwrap_or_else(|e| e.into_inner()) = app.map(|a| a.to_lowercase());
    }
}

/// Язык по названию раскладки: «Russian», «English (US)», «ru», «us»…
pub fn lang_of_layout_name(name: &str) -> Option<Lang> {
    let n = name.trim().to_lowercase();
    if n.contains("russian") || n.contains("рус") || n == "ru" || n.starts_with("ru(") {
        Some(Lang::Ru)
    } else if n.contains("english") || matches!(n.as_str(), "us" | "gb" | "en") || n.starts_with("us(") {
        Some(Lang::En)
    } else {
        None
    }
}

enum Backend {
    Hyprland(Hyprland),
    Kde(Kde),
    Unsupported,
}

pub struct Desktop {
    state: Arc<DesktopState>,
    backend: Backend,
}

impl Desktop {
    /// Определяет окружение по переменным среды и запускает слежение за ним.
    pub fn detect() -> Desktop {
        let state = Arc::new(DesktopState::default());
        let backend = if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            match Hyprland::start(state.clone()) {
                Some(h) => Backend::Hyprland(h),
                None => Backend::Unsupported,
            }
        } else if std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("KDE")) {
            match Kde::start(state.clone()) {
                Some(k) => Backend::Kde(k),
                None => Backend::Unsupported,
            }
        } else {
            Backend::Unsupported
        };
        Desktop { state, backend }
    }

    pub fn name(&self) -> &'static str {
        match self.backend {
            Backend::Hyprland(_) => "Hyprland",
            Backend::Kde(_) => "KDE Plasma",
            Backend::Unsupported => "не поддерживается",
        }
    }

    pub fn supported(&self) -> bool {
        !matches!(self.backend, Backend::Unsupported)
    }

    pub fn state(&self) -> Arc<DesktopState> {
        self.state.clone()
    }

    /// Языки, для которых в системе нет раскладки.
    pub fn missing(&self) -> Vec<Lang> {
        let have: Vec<Lang> = match &self.backend {
            Backend::Hyprland(h) => h.layouts().into_iter().flatten().collect(),
            Backend::Kde(k) => k.layouts().into_iter().flatten().collect(),
            Backend::Unsupported => return Vec::new(),
        };
        Lang::ALL.into_iter().filter(|l| !have.contains(l)).collect()
    }

    /// Переключает раскладку и дожидается подтверждения (до 150 мс): следом мы будем печатать
    /// физическими клавишами, и раскладка должна уже смениться.
    pub fn switch(&self, lang: Lang) -> bool {
        let ok = match &self.backend {
            Backend::Hyprland(h) => h.switch(lang),
            Backend::Kde(k) => k.switch(lang),
            Backend::Unsupported => false,
        };
        if ok {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(150);
            while self.state.lang() != Some(lang) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            self.state.set_lang(Some(lang));
        }
        ok
    }
}

// ---------------------------------------------------------------------------------------------
// Hyprland

struct Hyprland {
    dir: PathBuf,
}

impl Hyprland {
    fn start(state: Arc<DesktopState>) -> Option<Hyprland> {
        let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
        let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        let dir = [PathBuf::from(&runtime).join("hypr").join(&sig), PathBuf::from("/tmp/hypr").join(&sig)]
            .into_iter()
            .find(|d| d.join(".socket.sock").exists())?;
        let h = Hyprland { dir };

        // Начальное состояние.
        state.set_lang(h.active_lang());
        if let Some(w) = h.request_json("j/activewindow") {
            state.set_app(w["class"].as_str().map(str::to_string));
            state.window.store(parse_address(w["address"].as_str().unwrap_or("")), Ordering::Relaxed);
            state.fullscreen.store(is_fullscreen(&w), Ordering::Relaxed);
            state.pid.store(w["pid"].as_u64().unwrap_or(0) as u32, Ordering::Relaxed);
        }

        // События.
        let events = h.dir.join(".socket2.sock");
        let st = state.clone();
        let dir = h.dir.clone();
        std::thread::Builder::new()
            .name("switcher-hyprland".into())
            .spawn(move || {
                let Ok(stream) = UnixStream::connect(&events) else { return };
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    let Some((event, data)) = line.split_once(">>") else { continue };
                    match event {
                        "activelayout" => {
                            if let Some((_, layout)) = data.rsplit_once(',') {
                                st.set_lang(lang_of_layout_name(layout));
                            }
                        }
                        "activewindow" => {
                            let class = data.split(',').next().unwrap_or("");
                            st.set_app((!class.is_empty()).then(|| class.to_string()));
                        }
                        "activewindowv2" => {
                            st.window.store(parse_address(data), Ordering::Relaxed);
                            // У нового окна свой полноэкранный режим — спрашиваем его.
                            let w = Hyprland { dir: dir.clone() }.request_json("j/activewindow");
                            st.fullscreen.store(w.as_ref().is_some_and(is_fullscreen), Ordering::Relaxed);
                            st.pid.store(
                                w.as_ref().and_then(|w| w["pid"].as_u64()).unwrap_or(0) as u32,
                                Ordering::Relaxed,
                            );
                        }
                        // Срабатывает и на «развернуть» — уточняем у окна.
                        "fullscreen" => {
                            let w = Hyprland { dir: dir.clone() }.request_json("j/activewindow");
                            st.fullscreen.store(w.as_ref().is_some_and(is_fullscreen), Ordering::Relaxed);
                        }
                        _ => {}
                    }
                }
            })
            .ok()?;
        Some(h)
    }

    fn request(&self, cmd: &str) -> Option<String> {
        let mut s = UnixStream::connect(self.dir.join(".socket.sock")).ok()?;
        s.write_all(cmd.as_bytes()).ok()?;
        let mut out = String::new();
        s.read_to_string(&mut out).ok()?;
        Some(out)
    }

    fn request_json(&self, cmd: &str) -> Option<serde_json::Value> {
        serde_json::from_str(&self.request(cmd)?).ok()
    }

    /// Основная клавиатура из `hyprctl devices`.
    fn main_keyboard(&self) -> Option<serde_json::Value> {
        let devices = self.request_json("j/devices")?;
        let keyboards = devices["keyboards"].as_array()?;
        keyboards
            .iter()
            .find(|k| k["main"].as_bool() == Some(true))
            .or_else(|| keyboards.iter().find(|k| k["name"].as_str() != Some("switcher-virtual-keyboard")))
            .cloned()
    }

    fn active_lang(&self) -> Option<Lang> {
        lang_of_layout_name(self.main_keyboard()?["active_keymap"].as_str()?)
    }

    /// Языки раскладок по порядку («us,ru» → [En, Ru]).
    fn layouts(&self) -> Vec<Option<Lang>> {
        self.main_keyboard()
            .and_then(|k| k["layout"].as_str().map(|l| l.split(',').map(lang_of_layout_name).collect()))
            .unwrap_or_default()
    }

    fn switch(&self, lang: Lang) -> bool {
        let Some(index) = self.layouts().iter().position(|l| *l == Some(lang)) else { return false };
        if self.request(&format!("switchxkblayout all {index}")).is_some_and(|r| r.trim() == "ok") {
            return true;
        }
        // Старые версии Hyprland не знают «all» — переключаем основную клавиатуру по имени.
        let Some(name) = self.main_keyboard().and_then(|k| k["name"].as_str().map(str::to_string)) else {
            return false;
        };
        self.request(&format!("switchxkblayout {name} {index}")).is_some_and(|r| r.trim() == "ok")
    }
}

/// Полноэкранность окна из `j/activewindow`: в разных версиях Hyprland это bool или число
/// (битовая маска: 1 — развёрнуто, 2 — на весь экран; развёрнутое окно не считаем).
fn is_fullscreen(w: &serde_json::Value) -> bool {
    match &w["fullscreen"] {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_u64().is_some_and(|m| m & 2 != 0),
        _ => false,
    }
}

fn parse_address(s: &str) -> u64 {
    u64::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// KDE Plasma

struct Kde {
    proxy: zbus::blocking::Proxy<'static>,
}

impl Kde {
    fn start(state: Arc<DesktopState>) -> Option<Kde> {
        let conn = zbus::blocking::Connection::session().ok()?;
        let proxy =
            zbus::blocking::Proxy::new(&conn, "org.kde.keyboard", "/Layouts", "org.kde.KeyboardLayouts")
                .ok()?;
        let kde = Kde { proxy };
        let layouts = kde.layouts();
        let current: Option<u32> = kde.proxy.call("getLayout", &()).ok();
        state.set_lang(current.and_then(|i| layouts.get(i as usize).copied().flatten()));

        let listener = kde.proxy.clone();
        let st = state.clone();
        std::thread::Builder::new()
            .name("switcher-kde".into())
            .spawn(move || {
                let Ok(signals) = listener.receive_signal("layoutChanged") else { return };
                for msg in signals {
                    if let Ok(index) = msg.body().deserialize::<u32>() {
                        st.set_lang(layouts.get(index as usize).copied().flatten());
                    }
                }
            })
            .ok()?;
        Some(kde)
    }

    fn layouts(&self) -> Vec<Option<Lang>> {
        let list: Vec<(String, String, String)> = self.proxy.call("getLayoutsList", &()).unwrap_or_default();
        list.iter()
            .map(|(short, _, long)| lang_of_layout_name(short).or_else(|| lang_of_layout_name(long)))
            .collect()
    }

    fn switch(&self, lang: Lang) -> bool {
        let Some(index) = self.layouts().iter().position(|l| *l == Some(lang)) else { return false };
        self.proxy.call::<_, _, bool>("setLayout", &(index as u32)).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_names() {
        assert_eq!(lang_of_layout_name("Russian"), Some(Lang::Ru));
        assert_eq!(lang_of_layout_name("English (US)"), Some(Lang::En));
        assert_eq!(lang_of_layout_name("us"), Some(Lang::En));
        assert_eq!(lang_of_layout_name("ru"), Some(Lang::Ru));
        assert_eq!(lang_of_layout_name("German"), None);
    }

    #[test]
    fn fullscreen_modes() {
        use serde_json::json;
        assert!(is_fullscreen(&json!({ "fullscreen": 2 })));
        assert!(!is_fullscreen(&json!({ "fullscreen": 1 })), "развёрнутое — не полный экран");
        assert!(is_fullscreen(&json!({ "fullscreen": true })));
        assert!(!is_fullscreen(&json!({})));
    }

    #[test]
    fn addresses() {
        assert_eq!(parse_address("0x55d1c3a0"), 0x55d1c3a0);
        assert_eq!(parse_address("garbage"), 0);
    }
}
