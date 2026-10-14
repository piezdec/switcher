//! Контроллер живёт в потоке хука: решает, пропустить нажатие или проглотить, и ставит
//! задания рабочему потоку. Всё, что здесь происходит, должно укладываться в микросекунды.

use crate::config::{self, Config};
use crate::hotkeys::{Command, Hit, Matcher};
use crate::profiles;
use crate::sys::{KeyEvent, Sound, Sys, Verdict, VK_CAPITAL, VK_TAB};
use engine::keys::{classify, KeyClass};
use engine::translit::Style;
use engine::{Engine, Fix, KeyInput, Lang, Learning, Mode, Reason, Snippet, TextOptions};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Сколько доверять собственному представлению о раскладке после переключения,
/// пока система не подтвердила его (консоли и Electron отвечают с задержкой).
const ASSUMED_LAYOUT_TTL: Duration = Duration::from_millis(1000);

/// Что сделать с выделенным текстом.
#[derive(Clone, Copy, Debug)]
pub enum SelectionMode {
    /// Перекодировать в другую раскладку: «ghbdtn» → «привет».
    Layout,
    /// Транслитерировать: «Привет» ⇄ «Privet».
    Translit(Style),
}

pub enum Action {
    Fix(Fix),
    /// Нажатие, перехваченное, пока шло исправление; досылается после него в исходном порядке.
    Replay(KeyEvent),
    /// Перевести (или транслитерировать) выделенный текст через буфер обмена.
    Selection(SelectionMode),
    /// Записать файл (настройки, выученные слова) — диск не для потока хука.
    WriteFile {
        path: PathBuf,
        contents: String,
    },
    Quit,
}

#[derive(Default)]
pub struct Queue {
    /// Идёт исправление: все новые нажатия перехватываются и копятся в `actions`.
    pub pending: bool,
    pub actions: VecDeque<Action>,
}

/// Очередь между потоком хука и рабочим потоком.
#[derive(Default)]
pub struct Shared {
    pub queue: Mutex<Queue>,
    pub wake: Condvar,
}

impl Shared {
    pub fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Фоновое задание, не задерживающее ввод.
    pub fn push(&self, action: Action) {
        self.lock().actions.push_back(action);
        self.wake.notify_one();
    }

    pub fn quit(&self) {
        self.push(Action::Quit);
    }
}

/// Как часто проверять, не изменились ли файлы настроек.
const FILE_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// Слежение за одним файлом: время изменения и то, что мы записали сами.
#[derive(Default)]
struct WatchedFile {
    stamp: Option<std::time::SystemTime>,
    /// Содержимое последней нашей записи: свою же запись перечитывать незачем.
    written: String,
}

impl WatchedFile {
    fn new(path: &std::path::Path) -> Self {
        WatchedFile { stamp: config::modified(path), written: String::new() }
    }

    /// Новое содержимое файла, если он изменился и это не наша запись.
    fn changed(&mut self, path: &std::path::Path) -> Option<String> {
        let stamp = config::modified(path);
        if stamp == self.stamp {
            return None;
        }
        self.stamp = stamp;
        let text = std::fs::read_to_string(path).ok()?;
        (text != self.written).then_some(text)
    }
}

struct Files {
    config: WatchedFile,
    learned: WatchedFile,
    snippets: WatchedFile,
    checked: Instant,
}

/// Состояние модификаторов. Ведём сами по событиям хука: опрос ОС из хука ненадёжен.
#[derive(Default)]
struct Modifiers {
    shift: [bool; 2],
    ctrl: [bool; 2],
    alt: [bool; 2],
    win: [bool; 2],
    caps: bool,
    caps_down: bool,
    /// Какие клавиши сейчас зажаты (по VK) — чтобы отличить автоповтор от нового нажатия.
    pressed: [u64; 4],
    /// Последнее нажатие — автоповтор.
    repeat: bool,
}

impl Modifiers {
    fn update(&mut self, ev: &KeyEvent) {
        let down = !ev.up;
        // Повторное нажатие без отпускания — автоповтор зажатой клавиши.
        let slot = ev.vk as usize & 0xFF;
        let (word, bit) = (slot / 64, 1u64 << (slot % 64));
        self.repeat = down && (ev.raw.repeat || self.pressed[word] & bit != 0);
        if down {
            self.pressed[word] |= bit;
        } else {
            self.pressed[word] &= !bit;
        }
        match ev.vk {
            0xA0 | 0x10 => self.shift[0] = down,
            0xA1 => self.shift[1] = down,
            0xA2 | 0x11 => self.ctrl[0] = down,
            0xA3 => self.ctrl[1] = down,
            0xA4 | 0x12 => self.alt[0] = down,
            0xA5 => self.alt[1] = down,
            0x5B => self.win[0] = down,
            0x5C => self.win[1] = down,
            v if v == VK_CAPITAL => {
                // Автоповтор не должен переключать Caps Lock много раз.
                if down && !self.caps_down {
                    self.caps = !self.caps;
                }
                self.caps_down = down;
            }
            _ => {}
        }
    }

    fn input(&self, vk: u16) -> KeyInput {
        KeyInput {
            vk,
            shift: self.shift.iter().any(|&b| b),
            caps: self.caps,
            ctrl: self.ctrl.iter().any(|&b| b),
            alt: self.alt.iter().any(|&b| b),
            win: self.win.iter().any(|&b| b),
            repeat: self.repeat,
        }
    }
}

pub struct Controller {
    engine: Engine,
    mods: Modifiers,
    shared: Arc<Shared>,
    /// Сведения о системе (активное окно, раскладка, поле пароля).
    sys: Box<dyn Sys>,
    /// Раскладка, на которую мы сами переключили окно, и когда.
    assumed: Option<(Lang, Instant)>,
    foreground: u64,
    /// Исполняемый файл активного окна — для профилей и исключений.
    exe: Option<String>,
    /// Последняя программа, кроме нас самих: к ней относится «исключить» в меню.
    last_app: Option<String>,
    own_exe: Option<String>,
    config: Config,
    snippets: Vec<Snippet>,
    hotkeys: Matcher,
    files: Files,
    /// Активное окно на весь экран (игра, видео) — по таймеру и при смене окна.
    fullscreen: bool,
    /// Активное окно — игра из лаунчера (по пути к exe) — при смене окна.
    game: bool,
    /// Пометка для плашки у курсора («выучено») и когда она появилась.
    note: Option<(&'static str, Instant)>,
    /// Что последним записано в журнал о состоянии (пауза с причиной или работа).
    logged_state: Option<Option<String>>,
}

impl Controller {
    pub fn new(
        sys: Box<dyn Sys>,
        shared: Arc<Shared>,
        config: Config,
        learning: Learning,
        snippets: Vec<Snippet>,
    ) -> Self {
        let mods = Modifiers { caps: sys.caps_lock_on(), ..Default::default() };
        let own_exe = sys.own_app();
        let mut engine = Engine::new();
        engine.auto = config.auto;
        engine.learning = learning;
        let mut c = Controller {
            engine,
            mods,
            shared,
            sys,
            assumed: None,
            foreground: 0,
            exe: None,
            last_app: None,
            own_exe,
            hotkeys: Matcher::new(&config.hotkeys),
            snippets,
            note: None,
            logged_state: None,
            fullscreen: false,
            game: false,
            files: Files {
                config: WatchedFile::new(&config::config_path()),
                learned: WatchedFile::new(&config::learned_path()),
                snippets: WatchedFile::new(&config::snippets_path()),
                checked: Instant::now(),
            },
            config,
        };
        c.on_foreground_changed();
        c
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Меняет настройки и сохраняет их на диск (в рабочем потоке).
    pub fn update_config(&mut self, f: impl FnOnce(&mut Config)) {
        f(&mut self.config);
        self.apply_config();
        let contents = config::config_to_string(&self.config);
        self.files.config.written = contents.clone();
        self.shared.push(Action::WriteFile { path: config::config_path(), contents });
    }

    fn apply_config(&mut self) {
        self.engine.auto = self.config.auto;
        self.hotkeys.configure(&self.config.hotkeys);
        self.apply_mode();
    }

    /// Текстовые исправления для ядра. Заглавные в начале предложения — не в терминалах и IDE.
    fn apply_text_options(&mut self) {
        let dev = self.exe.as_deref().is_some_and(profiles::is_dev_tool);
        self.engine.text = TextOptions {
            snippets: self.snippets.clone(),
            fix_case: self.config.fix_case,
            capitalize_sentences: self.config.capitalize_sentences && !dev,
            fix_typos: self.config.fix_typos,
        };
    }

    /// Подхват правок файлов, сделанных не нами (окно настроек, ручная правка).
    fn reload_changed_files(&mut self) {
        if self.files.checked.elapsed() < FILE_CHECK_INTERVAL {
            return;
        }
        self.files.checked = Instant::now();
        if let Some(text) = self.files.config.changed(&config::config_path()) {
            if let Some(cfg) = config::parse_config(&text) {
                diag!("config.toml изменён снаружи — применяю");
                self.config = cfg;
                self.apply_config();
            }
        }
        if let Some(text) = self.files.snippets.changed(&config::snippets_path()) {
            if let Some(snippets) = config::parse_snippets(&text) {
                diag!("snippets.toml изменён снаружи — применяю");
                self.snippets = snippets;
                self.apply_text_options();
            }
        }
        if let Some(text) = self.files.learned.changed(&config::learned_path()) {
            if let Some(learning) = config::parse_learning(&text) {
                diag!("learned.toml изменён снаружи — применяю");
                self.engine.learning = learning;
            }
        }
    }

    /// Свежая пометка для плашки (не старше полутора секунд). Плашка есть только в Windows.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn take_note(&mut self) -> Option<&'static str> {
        self.note.take().filter(|(_, at)| at.elapsed() < Duration::from_millis(1500)).map(|(n, _)| n)
    }

    /// Программа, к которой относится пункт «исключить» в меню.
    pub fn last_app(&self) -> Option<&str> {
        self.last_app.as_deref()
    }

    pub fn is_excluded(&self, exe: &str) -> bool {
        self.config.excluded_apps.iter().any(|e| e == exe)
    }

    fn excluded_now(&self) -> bool {
        // Своё окно настроек тоже: иначе слово, вписываемое в словарь, переводилось бы на лету.
        (self.exe.is_some() && self.exe == self.own_exe)
            || self.exe.as_deref().is_some_and(|e| self.is_excluded(e))
    }

    /// Режим, который действует в активном окне с учётом профиля.
    pub fn effective_mode(&self) -> Mode {
        let dev = self.exe.as_deref().is_some_and(profiles::is_dev_tool);
        if self.config.careful_in_dev_tools && dev {
            Mode::Careful
        } else {
            self.config.mode.into()
        }
    }

    fn apply_mode(&mut self) {
        let mode = self.effective_mode();
        self.engine.set_mode(mode);
        self.apply_text_options();
    }

    /// Пауза в играх (на весь экран или запущенных из лаунчера) и полноэкранном видео,
    /// если она включена в настройках.
    fn paused_for_fullscreen(&self) -> bool {
        self.config.pause_in_fullscreen && (self.fullscreen || self.game)
    }

    /// Активно ли переключение в текущем окне (для значка).
    pub fn active_here(&self) -> bool {
        self.config.auto && !self.excluded_now() && !self.sys.in_password() && !self.paused_for_fullscreen()
    }

    /// Обработчик хука клавиатуры.
    pub fn on_key(&mut self, ev: &KeyEvent) -> Verdict {
        self.mods.update(ev);
        crate::update::touch_input();
        // Tab переводит фокус между полями — пусть наблюдатель проверит, не пароль ли там.
        if ev.vk == VK_TAB && !ev.up {
            self.sys.poke_password();
        }

        let m = self.mods.input(ev.vk);
        let hit = self.hotkeys.on_event(ev.vk, ev.up, m.ctrl, m.alt, m.shift, m.win);
        let blocked = self.sys.in_password() || self.excluded_now() || self.paused_for_fullscreen();
        match hit {
            Hit::SwallowUp => return Verdict::Swallow,
            Hit::Command { command, swallow } => {
                // Включать и выключать можно везде; переводить — только там, где мы работаем.
                if command == Command::ToggleAuto {
                    self.update_config(|cfg| cfg.auto = !cfg.auto);
                } else if !blocked {
                    self.run_command(command);
                }
                return if swallow { Verdict::Swallow } else { Verdict::Pass };
            }
            Hit::None => {}
        }

        // Поле пароля и исключённые программы: ничего не видим и не трогаем.
        if blocked {
            self.engine.reset();
            return Verdict::Pass;
        }

        let shared = Arc::clone(&self.shared);
        let mut q = shared.lock();

        if q.pending {
            // Пока рабочий поток исправляет слово, ввод нельзя пускать в окно, иначе
            // Backspace сотрёт не то. Ядро видит нажатия сразу, а в окно они уйдут позже.
            if !ev.up {
                let layout = self.layout_now(true);
                if let Some(fix) = self.engine.on_key(self.mods.input(ev.vk), layout) {
                    self.enqueue(&mut q, fix);
                    return Verdict::Swallow;
                }
            }
            q.actions.push_back(Action::Replay(*ev));
            return Verdict::Swallow;
        }

        if ev.up {
            return Verdict::Pass;
        }
        let layout = self.layout_now(false);
        if self.config.typing_click && layout == Some(Lang::Ru) && classify(ev.vk, m.shift) == KeyClass::Word
        {
            self.sys.play_sound(Sound::Click);
        }
        match self.engine.on_key(self.mods.input(ev.vk), layout) {
            Some(fix) => {
                self.enqueue(&mut q, fix);
                Verdict::Swallow
            }
            None => Verdict::Pass,
        }
    }

    fn run_command(&mut self, command: Command) {
        let shared = Arc::clone(&self.shared);
        let mut q = shared.lock();
        match command {
            Command::ConvertWord => {
                let layout = self.layout_now(q.pending);
                if let Some(fix) = self.engine.manual_convert(layout) {
                    self.enqueue(&mut q, fix);
                }
            }
            Command::ConvertSelection => {
                // Нажатия при этом не придерживаем (pending не ставим): иначе задержится и
                // отпускание модификаторов, система будет считать их зажатыми, и наше Ctrl+Insert
                // (копировать) превратится, например, в Ctrl+Shift+Insert (вставить).
                self.engine.reset();
                q.actions.push_back(Action::Selection(SelectionMode::Layout));
                self.shared.wake.notify_one();
            }
            Command::TransliterateSelection => {
                self.engine.reset();
                q.actions
                    .push_back(Action::Selection(SelectionMode::Translit(self.config.translit_style.into())));
                self.shared.wake.notify_one();
            }
            Command::ToggleAuto => {}
        }
    }

    pub fn on_click(&mut self) {
        self.engine.reset();
        self.sys.poke_password();
    }

    fn on_foreground_changed(&mut self) {
        self.sys.poke_password();
        self.foreground = self.sys.foreground_window();
        self.fullscreen = self.sys.is_fullscreen();
        self.game = self.sys.is_game();
        self.assumed = None;
        self.engine.reset_context();
        self.exe = self.sys.foreground_app();
        let shell = self.sys.is_shell();
        // Панель задач и трей не запоминаем: туда щёлкают, чтобы открыть меню Switcher.
        if self.exe.is_some() && self.exe != self.own_exe && !shell {
            self.last_app = self.exe.clone();
        }
        self.apply_mode();
        let mut flags = Vec::new();
        if self.fullscreen {
            flags.push("на весь экран");
        }
        if self.game {
            flags.push("игра по папке лаунчера");
        }
        if shell {
            flags.push("оболочка Windows");
        }
        diag!(
            "окно: {} [{}]{}",
            self.exe.as_deref().unwrap_or("?"),
            self.sys.describe(),
            if flags.is_empty() { String::new() } else { format!(" — {}", flags.join(", ")) }
        );
    }

    /// Почему переключатель сейчас не работает в активном окне (`None` — работает).
    /// Ровно то, что делает значок серым.
    pub fn pause_reason(&self, lang: Option<Lang>) -> Option<String> {
        let app = self.exe.as_deref().unwrap_or("?");
        if !self.config.auto {
            Some("автопереключение выключено".into())
        } else if self.exe.is_some() && self.exe == self.own_exe {
            Some("своё окно настроек".into())
        } else if self.exe.as_deref().is_some_and(|e| self.is_excluded(e)) {
            Some(format!("программа в исключениях: {app}"))
        } else if self.sys.in_password() {
            Some("поле пароля".into())
        } else if self.config.pause_in_fullscreen && self.fullscreen {
            Some(format!("активное окно на весь экран: {app} (настройка «Не работать в играх»)"))
        } else if self.config.pause_in_fullscreen && self.game {
            Some(format!("игра по папке лаунчера: {app} (настройка «Не работать в играх»)"))
        } else if lang.is_none() {
            Some(format!("раскладка окна не русская и не английская: {app}"))
        } else {
            None
        }
    }

    /// Периодическая проверка (по таймеру UI). Возвращает раскладку для значка.
    pub fn on_timer(&mut self) -> Option<Lang> {
        // Полноэкранность окна может смениться и без смены окна (F11, режим игры).
        self.fullscreen = self.sys.is_fullscreen();
        if self.sys.foreground_window() != self.foreground {
            self.on_foreground_changed();
        }
        if self.engine.learning.take_dirty() {
            let contents = config::learning_to_string(&self.engine.learning);
            self.files.learned.written = contents.clone();
            self.shared.push(Action::WriteFile { path: config::learned_path(), contents });
        }
        self.reload_changed_files();
        let observed = self.sys.foreground_lang();
        let lang = match self.assumed {
            Some((lang, at)) if observed != Some(lang) && at.elapsed() <= ASSUMED_LAYOUT_TTL => Some(lang),
            _ => {
                self.assumed = None;
                observed
            }
        };
        // В журнал — каждый переход «работает ↔ пауза» с причиной.
        let state = self.pause_reason(lang);
        if self.logged_state.as_ref() != Some(&state) {
            match &state {
                Some(why) => diag!("состояние: пауза — {why}"),
                None => diag!("состояние: работает (раскладка {})", lang.map_or("?", Lang::code)),
            }
            self.logged_state = Some(state);
        }
        lang
    }

    fn layout_now(&self, pending: bool) -> Option<Lang> {
        match self.assumed {
            Some((lang, at)) if pending || at.elapsed() < ASSUMED_LAYOUT_TTL => Some(lang),
            _ => self.sys.foreground_lang(),
        }
    }

    fn enqueue(&mut self, q: &mut Queue, fix: Fix) {
        debug_log!("fix: {:?}", fix);
        diag!("{}", describe_fix(&fix));
        if let Some(lang) = fix.switch_to {
            self.assumed = Some((lang, Instant::now()));
            if self.config.switch_sound {
                self.sys.play_sound(if lang == Lang::Ru { Sound::ToRussian } else { Sound::ToEnglish });
            }
        }
        if fix.decision.as_ref().is_some_and(|d| d.reason == Reason::Learned) {
            self.note = Some(("выучено", Instant::now()));
        }
        q.actions.push_back(Action::Fix(fix));
        q.pending = true;
        self.shared.wake.notify_one();
    }
}

/// Исправление для журнала — без самого текста: что за исправление, почему и сколько стёрто.
fn describe_fix(fix: &Fix) -> String {
    let what = if fix.expand_macros {
        "сокращение".to_string()
    } else if let Some(to) = fix.switch_to {
        format!("раскладка {}→{}", to.other(), to)
    } else if fix.caps_lock_off {
        "регистр и Caps Lock".to_string()
    } else {
        "регистр или опечатка".to_string()
    };
    let why = match &fix.decision {
        Some(d) => format!("{:?}", d.reason),
        None => "вручную или посреди слова".to_string(),
    };
    format!("исправление: {what}, причина {why}, стёрто {} симв.", fix.erase)
}
