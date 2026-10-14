//! Проверка слоя клавиатуры без настоящей клавиатуры (нужен доступ к /dev/uinput):
//! создаёт «физическую» тестовую клавиатуру, захватывает её как Switcher, нажимает на ней
//! клавиши и проверяет, что события дошли; затем пересылает их через виртуальную клавиатуру
//! Switcher и читает обратно уже с неё.
//!
//!   cargo run -p platform-linux --example evdev_loop

#[cfg(target_os = "linux")]
fn main() {
    use evdev::uinput::VirtualDevice;
    use evdev::{AttributeSet, EventType, InputEvent, KeyCode};
    use platform_linux::input::{self, Grabbed, Output, VIRTUAL_NAME};
    use platform_linux::keymap;
    use std::time::Duration;

    let mut keys = AttributeSet::<KeyCode>::new();
    for code in keymap::all_codes() {
        keys.insert(KeyCode(code));
    }
    let mut fake = VirtualDevice::builder()
        .and_then(|b| b.name("Fake physical keyboard").with_keys(&keys))
        .and_then(|b| b.build())
        .expect("нет доступа к /dev/uinput");
    let out = Output::create().expect("виртуальная клавиатура Switcher");
    std::thread::sleep(Duration::from_millis(300)); // udev создаёт узлы устройств

    let found = input::keyboards();
    let names: Vec<_> =
        found.iter().map(|(p, d)| format!("{} ({})", d.name().unwrap_or("?"), p.display())).collect();
    println!("клавиатуры: {names:?}");
    assert!(
        found.iter().all(|(_, d)| d.name() != Some(VIRTUAL_NAME)),
        "своя виртуальная клавиатура не должна считаться физической"
    );
    let (_, dev) = found
        .into_iter()
        .find(|(_, d)| d.name() == Some("Fake physical keyboard"))
        .expect("тестовая клавиатура найдена");
    let mut grabbed = Grabbed::grab(dev).expect("захват");
    println!("захвачена, Caps Lock: {}", grabbed.caps());

    // «Нажимаем» g, h и Caps Lock на «физической» клавиатуре.
    for code in [34u16, 35, keymap::KEY_CAPSLOCK] {
        fake.emit(&[InputEvent::new(EventType::KEY.0, code, 1)]).unwrap();
        fake.emit(&[InputEvent::new(EventType::KEY.0, code, 0)]).unwrap();
    }
    let mut got = Vec::new();
    while got.len() < 6 {
        got.extend(grabbed.read().expect("чтение"));
    }
    println!("прочитано: {got:?}");
    let downs: Vec<u16> = got.iter().filter(|e| e.value == 1).map(|e| e.code).collect();
    assert_eq!(downs, [34, 35, keymap::KEY_CAPSLOCK]);
    assert!(grabbed.caps(), "Caps Lock должен включиться");
    assert_eq!(keymap::vk_of(34), 0x47, "g → VK_G");

    // Пересылка через виртуальную клавиатуру Switcher: открываем её как устройство и читаем.
    let (_, mut virt) =
        evdev::enumerate().find(|(_, d)| d.name() == Some(VIRTUAL_NAME)).expect("виртуальная видна");
    out.tap_with_shift(keymap::code_of(0x47).unwrap(), true); // Shift+G
    let mut echoed = Vec::new();
    while echoed.len() < 4 {
        echoed.extend(
            virt.fetch_events()
                .unwrap()
                .filter(|e| e.event_type() == EventType::KEY)
                .map(|e| (e.code(), e.value())),
        );
    }
    println!("с виртуальной клавиатуры: {echoed:?}");
    assert_eq!(echoed, [(keymap::KEY_LEFTSHIFT, 1), (34, 1), (34, 0), (keymap::KEY_LEFTSHIFT, 0)]);
    println!("OK: захват, чтение, Caps Lock и пересылка работают");
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Пример только для Linux.");
}
