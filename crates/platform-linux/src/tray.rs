//! Значок в трее через StatusNotifierItem (KDE — сразу, Hyprland — через waybar с модулем tray).

use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Icon, MenuItem, ToolTip};
use std::sync::mpsc::Sender;

/// Что показывать в трее.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayView {
    /// «EN», «RU» или «--».
    pub label: String,
    /// Переключение работает в активном окне (оранжевый значок) или нет (серый).
    pub active: bool,
    pub auto: bool,
    /// Строки-подсказки о горячих клавишах.
    pub hints: Vec<String>,
    /// Программа активного окна — для пункта «не переключать в …».
    pub app: Option<String>,
    pub app_excluded: bool,
    pub autostart: bool,
    pub tooltip: String,
    /// Версия, которая скачана и ждёт перезапуска.
    pub update: Option<String>,
}

/// Команда из меню.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    RestartToUpdate,
    ToggleAuto,
    ToggleExcludeApp,
    ToggleAutostart,
    OpenSettings,
    OpenDictionary,
    OpenDataDir,
    OpenLog,
    Quit,
}

struct Tray {
    view: TrayView,
    commands: Sender<TrayCommand>,
}

impl Tray {
    fn send(&self, c: TrayCommand) {
        let _ = self.commands.send(c);
    }
}

fn item(label: &str, command: TrayCommand) -> MenuItem<Tray> {
    StandardItem {
        label: label.into(),
        activate: Box::new(move |t: &mut Tray| t.send(command.clone())),
        ..Default::default()
    }
    .into()
}

fn check(label: &str, checked: bool, command: TrayCommand) -> MenuItem<Tray> {
    CheckmarkItem {
        label: label.into(),
        checked,
        activate: Box::new(move |t: &mut Tray| t.send(command.clone())),
        ..Default::default()
    }
    .into()
}

fn hint(label: &str) -> MenuItem<Tray> {
    StandardItem { label: label.into(), enabled: false, ..Default::default() }.into()
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "switcher".into()
    }

    fn title(&self) -> String {
        "Switcher".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![render_icon(&self.view.label, self.view.active)]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: "Switcher".into(), description: self.view.tooltip.clone(), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::ToggleAuto);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let v = &self.view;
        let mut m = vec![check("Автопереключение", v.auto, TrayCommand::ToggleAuto)];
        if let Some(app) = &v.app {
            m.push(check(&format!("Не переключать в {app}"), v.app_excluded, TrayCommand::ToggleExcludeApp));
        }
        m.push(MenuItem::Separator);
        m.extend(v.hints.iter().map(|h| hint(h)));
        m.push(MenuItem::Separator);
        m.push(check("Запускать при входе в систему", v.autostart, TrayCommand::ToggleAutostart));
        if let Some(tag) = &v.update {
            m.push(item(&format!("Перезапустить и обновить до {tag}"), TrayCommand::RestartToUpdate));
        }
        m.push(item("Выученные слова…", TrayCommand::OpenDictionary));
        m.push(item("Настройки…", TrayCommand::OpenSettings));
        m.push(item("Открыть папку настроек", TrayCommand::OpenDataDir));
        m.push(item("Открыть журнал работы", TrayCommand::OpenLog));
        m.push(item("Выход", TrayCommand::Quit));
        m
    }
}

/// Работающий значок; обновляется из любого потока.
pub struct TrayHandle(Option<ksni::blocking::Handle<Tray>>);

impl TrayHandle {
    /// Запускает значок. Если панели с треем нет, программа работает и без него.
    pub fn start(view: TrayView, commands: Sender<TrayCommand>) -> TrayHandle {
        match (Tray { view, commands }).spawn() {
            Ok(h) => TrayHandle(Some(h)),
            Err(e) => {
                eprintln!("switcher: значок в трее недоступен ({e}). Работаю без него.");
                TrayHandle(None)
            }
        }
    }

    pub fn update(&self, view: TrayView) {
        if let Some(h) = &self.0 {
            h.update(|t| t.view = view);
        }
    }
}

// --- значок -----------------------------------------------------------------------------------

/// Пиксельный шрифт 5×7 для нужных символов.
fn glyph(c: char) -> [u8; 7] {
    match c {
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        _ => [0, 0, 0, 0b11111, 0, 0, 0],
    }
}

/// Значок 32×32: оранжевый (или серый) квадрат с двумя буквами, ARGB32 в сетевом порядке байт.
pub fn render_icon(label: &str, active: bool) -> Icon {
    const SIZE: usize = 32;
    const SCALE: usize = 2;
    let bg: [u8; 4] = if active { [0xFF, 0xF2, 0x8C, 0x28] } else { [0xFF, 0x9A, 0x9A, 0x9A] };
    let fg: [u8; 4] = [0xFF, 0x00, 0x00, 0x00];
    let mut data = Vec::with_capacity(SIZE * SIZE * 4);
    for _ in 0..SIZE * SIZE {
        data.extend_from_slice(&bg);
    }
    let chars: Vec<char> = label.chars().take(2).collect();
    let text_w = chars.len() * 5 * SCALE + chars.len().saturating_sub(1) * SCALE;
    let x0 = (SIZE - text_w) / 2;
    let y0 = (SIZE - 7 * SCALE) / 2;
    for (i, &c) in chars.iter().enumerate() {
        let gx = x0 + i * 6 * SCALE;
        for (row, bits) in glyph(c).iter().enumerate() {
            for col in 0..5 {
                if bits & (0b10000 >> col) != 0 {
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            let (x, y) = (gx + col * SCALE + dx, y0 + row * SCALE + dy);
                            let p = (y * SIZE + x) * 4;
                            data[p..p + 4].copy_from_slice(&fg);
                        }
                    }
                }
            }
        }
    }
    Icon { width: SIZE as i32, height: SIZE as i32, data }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_size_and_letters() {
        let icon = render_icon("RU", true);
        assert_eq!(icon.data.len(), 32 * 32 * 4);
        // Чёрные пиксели букв есть, фон — оранжевый.
        assert!(icon.data.chunks(4).any(|p| p == [0xFF, 0, 0, 0]));
        assert_eq!(&icon.data[..4], &[0xFF, 0xF2, 0x8C, 0x28]);
    }
}
