//! Клавиатура через evdev и uinput.
//!
//! Wayland не даёт программам читать чужой ввод, поэтому Switcher забирает физические
//! клавиатуры себе (`EVIOCGRAB`) и пересылает события через свою виртуальную клавиатуру,
//! придерживая те, что нужно. Так же работают keyd и kmonad. Если процесс завершится,
//! система сама вернёт клавиатуры — они не «зависнут».
//!
//! Нужны права: пользователь в группе `input` и доступ к `/dev/uinput` (см. README).

use crate::keymap;
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, Device, EventType, InputEvent, KeyCode, LedCode};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Имя нашей виртуальной клавиатуры — чтобы не схватить её саму.
pub const VIRTUAL_NAME: &str = "Switcher virtual keyboard";

/// Нажатие клавиши: `value` — 0 отпускание, 1 нажатие, 2 автоповтор.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawKeyEvent {
    pub code: u16,
    pub value: i32,
}

fn is_keyboard(dev: &Device) -> bool {
    dev.name() != Some(VIRTUAL_NAME)
        && dev
            .supported_keys()
            .is_some_and(|k| k.contains(KeyCode(keymap::KEY_A)) && k.contains(KeyCode(keymap::KEY_SPACE)))
}

fn is_mouse(dev: &Device) -> bool {
    dev.supported_keys().is_some_and(|k| k.contains(KeyCode::BTN_LEFT))
}

/// Физические клавиатуры.
pub fn keyboards() -> Vec<(PathBuf, Device)> {
    evdev::enumerate().filter(|(_, d)| is_keyboard(d)).collect()
}

/// Мыши и тачпады — только чтобы замечать щелчки (каретка могла уехать). Их не забираем.
pub fn mice() -> Vec<(PathBuf, Device)> {
    evdev::enumerate().filter(|(_, d)| is_mouse(d) && !is_keyboard(d)).collect()
}

/// Включён ли Caps Lock на клавиатуре (по её индикатору).
pub fn caps_lock_led(dev: &Device) -> bool {
    dev.get_led_state().is_ok_and(|leds| leds.contains(LedCode::LED_CAPSL))
}

/// Виртуальная клавиатура, через которую идёт весь ввод. Общая для потока перехвата
/// (пересылает нажатия) и рабочего потока (печатает исправления).
#[derive(Clone)]
pub struct Output {
    dev: Arc<Mutex<VirtualDevice>>,
}

impl Output {
    pub fn create() -> io::Result<Output> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for code in keymap::all_codes() {
            keys.insert(KeyCode(code));
        }
        let dev = VirtualDevice::builder()?.name(VIRTUAL_NAME).with_keys(&keys)?.build()?;
        Ok(Output { dev: Arc::new(Mutex::new(dev)) })
    }

    fn emit(&self, events: &[InputEvent]) {
        let mut dev = self.dev.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = dev.emit(events) {
            eprintln!("switcher: не удалось отправить нажатие: {e}");
        }
    }

    /// Переслать событие как есть.
    pub fn forward(&self, ev: RawKeyEvent) {
        self.emit(&[InputEvent::new(EventType::KEY.0, ev.code, ev.value)]);
    }

    /// Нажать и отпустить клавишу `count` раз.
    pub fn tap(&self, code: u16, count: usize) {
        for _ in 0..count {
            self.emit(&[InputEvent::new(EventType::KEY.0, code, 1)]);
            self.emit(&[InputEvent::new(EventType::KEY.0, code, 0)]);
        }
    }

    /// Нажать клавишу с Shift или без.
    pub fn tap_with_shift(&self, code: u16, shift: bool) {
        if shift {
            self.emit(&[InputEvent::new(EventType::KEY.0, keymap::KEY_LEFTSHIFT, 1)]);
        }
        self.tap(code, 1);
        if shift {
            self.emit(&[InputEvent::new(EventType::KEY.0, keymap::KEY_LEFTSHIFT, 0)]);
        }
    }

    /// Сочетание: модификаторы вниз, клавиша, модификаторы вверх.
    pub fn chord(&self, modifiers: &[u16], code: u16) {
        for &m in modifiers {
            self.emit(&[InputEvent::new(EventType::KEY.0, m, 1)]);
        }
        self.tap(code, 1);
        for &m in modifiers.iter().rev() {
            self.emit(&[InputEvent::new(EventType::KEY.0, m, 0)]);
        }
    }
}

/// Захваченная физическая клавиатура: читает события и зажигает индикатор Caps Lock
/// (виртуальная клавиатура индикаторов не имеет, и система сама его не обновит).
pub struct Grabbed {
    dev: Device,
    caps: bool,
}

impl Grabbed {
    pub fn grab(mut dev: Device) -> io::Result<Grabbed> {
        let caps = caps_lock_led(&dev);
        dev.grab()?;
        Ok(Grabbed { dev, caps })
    }

    pub fn caps(&self) -> bool {
        self.caps
    }

    /// Блокирующее чтение пачки событий клавиш.
    pub fn read(&mut self) -> io::Result<Vec<RawKeyEvent>> {
        let events: Vec<RawKeyEvent> = self
            .dev
            .fetch_events()?
            .filter(|e| e.event_type() == EventType::KEY)
            .map(|e| RawKeyEvent { code: e.code(), value: e.value() })
            .collect();
        for e in &events {
            if e.code == keymap::KEY_CAPSLOCK && e.value == 1 {
                self.caps = !self.caps;
                let led = InputEvent::new(EventType::LED.0, LedCode::LED_CAPSL.0, self.caps as i32);
                let _ = self.dev.send_events(&[led]);
            }
        }
        Ok(events)
    }
}

/// Мышь: блокирующее ожидание щелчка.
pub struct Mouse(Device);

impl Mouse {
    pub fn new(dev: Device) -> Mouse {
        Mouse(dev)
    }

    /// Ждёт следующего нажатия кнопки мыши.
    pub fn wait_click(&mut self) -> io::Result<()> {
        loop {
            let clicked = self.0.fetch_events()?.any(|e| {
                e.event_type() == EventType::KEY && e.value() == 1 && (0x110..=0x117).contains(&e.code())
            });
            if clicked {
                return Ok(());
            }
        }
    }
}
