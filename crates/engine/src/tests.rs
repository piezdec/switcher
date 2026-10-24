//! Сценарные тесты ядра: «печатаем» строку физическими клавишами и смотрим на исправления.

use super::*;
use keys::{VK_BACK, VK_SPACE};

/// Нажатия, которыми строка `text` набирается в раскладке `lang`.
fn strokes(text: &str, lang: Lang) -> Vec<KeyInput> {
    text.chars()
        .map(|c| {
            let (vk, shift) = match c {
                ' ' => (VK_SPACE, false),
                '\n' => (VK_RETURN, false),
                '\t' => (VK_TAB, false),
                '\u{8}' => (VK_BACK, false),
                _ => layouts::locate(c, lang).unwrap_or_else(|| panic!("нет клавиши для {c:?}")),
            };
            KeyInput { vk, shift, ..Default::default() }
        })
        .collect()
}

/// Набирает текст в раскладке `lang` и возвращает все исправления. Раскладка окна
/// после исправления меняется так же, как в реальной системе.
fn type_text(engine: &mut Engine, text: &str, mut lang: Lang) -> Vec<Fix> {
    let mut fixes = Vec::new();
    for k in strokes(text, lang) {
        if let Some(fix) = engine.on_key(k, Some(lang)) {
            lang = fix.switch_to.unwrap_or(lang);
            fixes.push(fix);
        }
    }
    fixes
}

/// Ядро без раннего переключения: исправление только на границе слова.
fn late() -> Engine {
    let mut e = Engine::new();
    e.params.early = false;
    e
}

fn late_fixes_for(text: &str, lang: Lang) -> Vec<String> {
    type_text(&mut late(), text, lang).into_iter().map(|f| f.text).collect()
}

fn fixes_for(text: &str, lang: Lang) -> Vec<String> {
    type_text(&mut Engine::new(), text, lang).into_iter().map(|f| f.text).collect()
}

#[test]
fn switches_latin_typo_to_russian() {
    let fixes = type_text(&mut late(), "ghbdtn ", Lang::En);
    assert_eq!(fixes.len(), 1);
    let f = &fixes[0];
    assert_eq!(
        (f.erase, f.text.as_str(), f.tail, f.switch_to),
        (6, "привет", Tail::Char(' '), Some(Lang::Ru))
    );
}

#[test]
fn switches_cyrillic_typo_to_english() {
    assert_eq!(fixes_for("руддщ ", Lang::Ru), ["hello"]);
}

#[test]
fn keeps_case() {
    assert_eq!(late_fixes_for("Ghbdtn ", Lang::En), ["Привет"]);
}

#[test]
fn russian_letters_on_punctuation_keys() {
    assert_eq!(fixes_for("nfr;t ", Lang::En), ["также"]);
    assert_eq!(fixes_for("k.lb ", Lang::En), ["люди"]);
}

#[test]
fn short_words() {
    assert_eq!(fixes_for("b ", Lang::En), ["и"]);
    assert_eq!(fixes_for("yt ", Lang::En), ["не"]);
    assert_eq!(fixes_for("ше ", Lang::Ru), ["it"]);
    assert!(fixes_for("a ", Lang::En).is_empty());
    assert!(fixes_for("I ", Lang::En).is_empty());
}

#[test]
fn whole_phrase_typed_in_wrong_layout() {
    // «привет как дела» набрано в английской раскладке: после первого слова окно уже русское.
    let mut e = late();
    let mut lang = Lang::En;
    let mut out = Vec::new();
    for k in strokes("ghbdtn ", lang) {
        if let Some(f) = e.on_key(k, Some(lang)) {
            lang = f.switch_to.unwrap_or(lang);
            out.push(f.text);
        }
    }
    assert_eq!(out, ["привет"]);
    assert_eq!(lang, Lang::Ru);
}

#[test]
fn enter_is_resent_as_key() {
    let fixes = type_text(&mut late(), "ghbdtn\n", Lang::En);
    assert_eq!(fixes[0].tail, Tail::Key(VK_RETURN));
}

#[test]
fn boundary_is_retyped_in_target_layout() {
    // `?` на английской раскладке — это запятая на русской.
    let fixes = type_text(&mut late(), "ghbdtn?", Lang::En);
    assert_eq!(fixes[0].tail, Tail::Char(','));
}

#[test]
fn leaves_english_alone() {
    let text = "hello world this is a test of the keyboard switcher strength beautiful rhythm ";
    assert!(fixes_for(text, Lang::En).is_empty(), "{:?}", fixes_for(text, Lang::En));
}

#[test]
fn leaves_russian_alone() {
    let text = "привет как дела сегодня шкаф холодильник программирование взгляд также люди ";
    assert!(fixes_for(text, Lang::Ru).is_empty(), "{:?}", fixes_for(text, Lang::Ru));
}

#[test]
fn leaves_technical_tokens_alone() {
    let text = "git status cargo build user_name a=b host:port win32 API camelCase ls cd ";
    assert!(fixes_for(text, Lang::En).is_empty(), "{:?}", fixes_for(text, Lang::En));
}

#[test]
fn auto_off_still_tracks_words() {
    let mut e = Engine { auto: false, ..Engine::new() };
    assert!(type_text(&mut e, "ghbdtn ", Lang::En).is_empty());
    let f = e.manual_convert(Some(Lang::En)).unwrap();
    assert_eq!((f.erase, f.text.as_str()), (7, "привет "));
}

#[test]
fn manual_convert_current_word_and_toggle_back() {
    let mut e = Engine::new();
    type_text(&mut e, "hello", Lang::En);
    let f = e.manual_convert(Some(Lang::En)).unwrap();
    assert_eq!((f.erase, f.text.as_str(), f.switch_to), (5, "руддщ", Some(Lang::Ru)));
    let back = e.manual_convert(Some(Lang::Ru)).unwrap();
    assert_eq!(back.text, "hello");
}

#[test]
fn manual_convert_last_word_toggles() {
    let mut e = Engine { auto: false, ..Engine::new() };
    type_text(&mut e, "ghbdtn ", Lang::En);
    assert_eq!(e.manual_convert(Some(Lang::En)).unwrap().text, "привет ");
    assert_eq!(e.manual_convert(Some(Lang::Ru)).unwrap().text, "ghbdtn ");
}

#[test]
fn manual_convert_not_after_enter() {
    let mut e = Engine { auto: false, ..Engine::new() };
    type_text(&mut e, "ghbdtn\n", Lang::En);
    assert!(e.manual_convert(Some(Lang::En)).is_none());
}

#[test]
fn backspace_edits_word() {
    // «ghbdtnx», стёрли x — получилось ghbdtn.
    assert_eq!(late_fixes_for("ghbdtnx\u{8} ", Lang::En), ["привет"]);
}

#[test]
fn backspace_over_boundary_resumes_last_word() {
    let mut e = Engine { auto: false, ..Engine::new() };
    type_text(&mut e, "hel \u{8}lo", Lang::En);
    assert_eq!(render_word(e.current_word(), Lang::En), "hello");
}

#[test]
fn shortcuts_reset_buffer() {
    let mut e = Engine::new();
    type_text(&mut e, "ghb", Lang::En);
    e.on_key(KeyInput { vk: 0x41, ctrl: true, ..Default::default() }, Some(Lang::En));
    assert!(e.current_word().is_empty());
}

#[test]
fn unknown_layout_never_auto_fixes() {
    let mut e = Engine::new();
    for k in strokes("ghbdtn ", Lang::En) {
        assert!(e.on_key(k, None).is_none());
    }
    // Но ручной перевод угадывает направление.
    assert_eq!(e.manual_convert(None).unwrap().text, "привет ");
}

#[test]
fn command_line_flags_are_left_alone() {
    assert!(fixes_for("git checkout -b feature ", Lang::En).is_empty());
    assert!(fixes_for("docker compose up -d ", Lang::En).is_empty());
    assert!(fixes_for("ls --all ", Lang::En).is_empty());
    // А дефис внутри слова — нет: «что-то» в английской раскладке.
    // После «что» окно уже русское, и «то» сразу печатается правильно.
    assert_eq!(fixes_for("xnj-nj ", Lang::En), ["что"]);
}

#[test]
fn english_contractions_are_english() {
    assert!(fixes_for("let's don't I'm won't ", Lang::En).is_empty());
}

#[test]
fn edge_letters_count_as_letters() {
    // Это, Будь, моё — с буквами на клавишах знаков по краям.
    assert_eq!(fixes_for("\"nj ", Lang::En), ["Это"]);
    assert_eq!(fixes_for("<elm ", Lang::En), ["Будь"]);
}

#[test]
fn closing_quote_is_not_camel_case() {
    // Закрывающая кавычка в русской раскладке — заглавная «Э»: `ЭДшеукфдднЭ` — не camelCase.
    assert_eq!(fixes_for("ЭДшеукфдднЭ ", Lang::Ru), ["\"Literally\""]);
    assert_eq!(fixes_for("рукуюЭ ", Lang::Ru), ["here.\""]);
    // А настоящий camelCase по-прежнему не трогаем.
    assert!(fixes_for("camelCase myValue ", Lang::En).is_empty());
}

#[test]
fn web_address_switches_before_space() {
    // «www.» и домен с известной зоной видны сразу — исправляем, не дожидаясь пробела.
    let fixes = type_text(&mut Engine::new(), "цццюпщщпдуюсщь", Lang::Ru);
    assert_eq!(fixes.first().map(|f| (f.text.as_str(), f.switch_to)), Some(("www.", Some(Lang::En))));
    // До пробела адрес так и остаётся английским.
    let fixes = type_text(&mut Engine::new(), "цццюпщщпдуюсщь ", Lang::Ru);
    assert!(fixes.iter().all(|f| f.switch_to != Some(Lang::Ru)), "{fixes:?}");
    // Короткое имя обычное раннее переключение не ловит (меньше 4 букв) — ловит зона.
    let fixes = type_text(&mut Engine::new(), "нфюкг", Lang::Ru);
    assert_eq!(fixes.first().map(|f| (f.text.as_str(), f.switch_to)), Some(("ya.ru", Some(Lang::En))));
    // «июля» в английской раскладке — `b.kz`: зону .kz не берём.
    assert!(fixes_for("июля ", Lang::Ru).is_empty());
    // Набранное в своей раскладке не трогаем.
    assert!(fixes_for("www.google.com ", Lang::En).is_empty());
}

#[test]
fn html_tags_are_left_alone() {
    // Короткие теги без «>»: `<td` — не «Бев». Но `<elm` — по-прежнему «Будь».
    assert!(fixes_for("<td <tr <li <th <ul ", Lang::En).is_empty());
    assert_eq!(fixes_for("<elm ", Lang::En), ["Будь"]);
}

#[test]
fn inner_apostrophe_needs_a_real_russian_word() {
    // Апостроф внутри английского слова — переводим, только если выходит словарное русское слово.
    assert!(fixes_for("rock'n'roll y'all ", Lang::En).is_empty());
    assert_eq!(fixes_for("gj'n ", Lang::En), ["поэт"]);
}

#[test]
fn russian_dot_abbreviations() {
    // «т.е.» в английской раскладке — `n/t/`: косая черта тут не путь, а русская точка.
    assert_eq!(fixes_for("n/t/ ", Lang::En), ["т.е."]);
    for (typed, want) in [("n/r/ ", "т.к."), ("n/l/ ", "т.д."), ("n/g/? ", "т.п.")] {
        assert_eq!(fixes_for(typed, Lang::En), [want]);
    }
    // Настоящие пути и n/a не трогаем.
    assert!(fixes_for("n/a src/lib ", Lang::En).is_empty());
}

#[test]
fn unknown_typed_but_known_abbreviation() {
    // Набранного нет в словаре ни в каком виде, а в другой раскладке — известное сокращение.
    assert_eq!(fixes_for("vaw ", Lang::En), ["мфц"]);
    assert_eq!(fixes_for("ylak ", Lang::En), ["ндфл"]);
}

#[test]
fn abbreviations_are_left_alone() {
    // Сокращения и единицы из tools/trainer/data/*_extra.txt — в любом регистре.
    assert!(fixes_for("СПб мск мфц ГВт гдж ндфл ", Lang::Ru).is_empty());
    assert!(fixes_for("kbps rtf ieee ", Lang::En).is_empty());
}

#[test]
fn numbers_are_left_alone() {
    assert!(fixes_for("127.0.0.1 3.14 ", Lang::En).is_empty());
}

#[test]
fn early_switch_mid_word() {
    // После «ghbd» уже ясно, что это «прив…»: исправляем, не дожидаясь пробела.
    let fixes = type_text(&mut Engine::new(), "ghbdtn ", Lang::En);
    assert_eq!(fixes.len(), 1);
    let f = &fixes[0];
    assert_eq!((f.erase, f.text.as_str(), f.tail, f.switch_to), (3, "прив", Tail::None, Some(Lang::Ru)));
}

#[test]
fn early_switch_waits_for_real_prefixes() {
    // «hell» — начало английского слова: раннего переключения нет.
    assert!(fixes_for("hello ", Lang::En).is_empty());
    assert!(fixes_for("привет ", Lang::Ru).is_empty());
}

#[test]
fn learns_from_manual_convert() {
    // «гига» модель не узнаёт; после двух ручных переводов — переводит сама.
    let mut e = late();
    for _ in 0..2 {
        assert!(type_text(&mut e, "ubuf ", Lang::En).is_empty());
        assert_eq!(e.manual_convert(Some(Lang::En)).unwrap().text, "гига ");
        e.reset();
    }
    let fixes = type_text(&mut e, "ubuf ", Lang::En);
    assert_eq!(fixes.len(), 1);
    assert_eq!(fixes[0].text, "гига");
    assert_eq!(fixes[0].decision.as_ref().unwrap().reason, Reason::Learned);
}

#[test]
fn learned_word_switches_without_space() {
    // Pause вместо пробела (адресная строка): дважды перевели `фвы` → `ads` — дальше сразу.
    let mut e = Engine::new();
    for _ in 0..2 {
        type_text(&mut e, "фвы", Lang::Ru);
        assert_eq!(e.manual_convert(Some(Lang::Ru)).unwrap().text, "ads");
        type_text(&mut e, " ", Lang::En);
        e.reset();
    }
    let fixes = type_text(&mut e, "фвы", Lang::Ru);
    assert_eq!(fixes.len(), 1);
    let f = &fixes[0];
    assert_eq!((f.erase, f.text.as_str(), f.switch_to), (2, "ads", Some(Lang::En)));
    // Дописанное после переключения остаётся английским.
    e.reset();
    type_text(&mut e, "фвы", Lang::Ru);
    let fixes = type_text(&mut e, "ense ", Lang::En);
    assert!(fixes.iter().all(|f| f.switch_to != Some(Lang::Ru)), "{fixes:?}");
}

#[test]
fn learned_word_waits_if_it_starts_a_real_word() {
    // Выученное «ру» (клавиши `he`) — начало «hello»: посреди слова не трогаем.
    let mut e = Engine::new();
    e.learning.learn(&keys_of("he"), Lang::Ru, 10);
    assert!(type_text(&mut e, "hello ", Lang::En).is_empty());
    // Одиночную букву выученное тоже не переключает до конца слова.
    let mut e = Engine::new();
    e.learning.learn(&keys_of("f"), Lang::Ru, 10);
    assert!(type_text(&mut e, "f", Lang::En).is_empty());
}

#[test]
fn undo_of_auto_switch_is_remembered_at_once() {
    let mut e = late();
    assert_eq!(type_text(&mut e, "ghbdtn ", Lang::En).len(), 1);
    // Pause сразу после переключения — отмена.
    assert_eq!(e.manual_convert(Some(Lang::Ru)).unwrap().text, "ghbdtn ");
    e.reset();
    assert!(type_text(&mut e, "ghbdtn ", Lang::En).is_empty());
}

#[test]
fn undo_of_early_switch_is_remembered() {
    let mut e = Engine::new();
    // «ghbd» → «прив» посреди слова, Pause возвращает, слово дописывается латиницей.
    let fixes = type_text(&mut e, "ghbd", Lang::En);
    assert_eq!(fixes.len(), 1);
    assert_eq!(e.manual_convert(Some(Lang::Ru)).unwrap().text, "ghbd");
    type_text(&mut e, "tn ", Lang::En);
    e.reset();
    assert!(type_text(&mut e, "ghbdtn ", Lang::En).is_empty(), "ни раннего, ни обычного переключения");
}

#[test]
fn pause_back_and_forth_learns_nothing() {
    let mut e = late();
    type_text(&mut e, "ubuf ", Lang::En);
    e.manual_convert(Some(Lang::En));
    e.manual_convert(Some(Lang::Ru));
    assert_eq!(e.learning.entries().count(), 0);
}

#[test]
fn no_manual_convert_after_tab() {
    let mut e = late();
    type_text(&mut e, "ubuf\t", Lang::En);
    assert!(e.manual_convert(Some(Lang::En)).is_none());
}

fn with_text(options: TextOptions) -> Engine {
    let mut e = late();
    e.text = options;
    e
}

#[test]
fn snippet_expands_in_either_layout() {
    let snippets = vec![Snippet { trigger: "адр".into(), text: "г. Москва{cursor}".into() }];
    let mut e = with_text(TextOptions { snippets, ..Default::default() });
    let fixes = type_text(&mut e, "адр ", Lang::Ru);
    assert_eq!(fixes.len(), 1);
    let f = &fixes[0];
    assert_eq!(
        (f.erase, f.text.as_str(), f.switch_to, f.expand_macros),
        (3, "г. Москва{cursor}", None, true)
    );
    assert_eq!(f.tail, Tail::Char(' '));
    // То же сокращение, набранное в английской раскладке (fhl).
    let fixes = type_text(&mut e, "fl[ ", Lang::En);
    assert!(fixes.is_empty(), "«fl[» — это «адх», а не «адр»");
    let fixes = type_text(&mut e, "fhl ", Lang::En);
    assert!(fixes.is_empty(), "«fhl» — это «ард»");
    let fixes = type_text(&mut e, "fkh ", Lang::En);
    assert!(fixes.is_empty());
    let fixes = type_text(&mut e, "flh ", Lang::En);
    assert_eq!(fixes[0].text, "г. Москва{cursor}");
}

#[test]
fn double_capital_is_fixed() {
    let mut e = with_text(TextOptions { fix_case: true, ..Default::default() });
    let fixes = type_text(&mut e, "ПРивет ", Lang::Ru);
    assert_eq!(fixes.len(), 1);
    assert_eq!((fixes[0].text.as_str(), fixes[0].switch_to), ("Привет", None));
}

#[test]
fn caps_lock_typo_is_fixed_and_caps_turned_off() {
    let mut e = with_text(TextOptions { fix_case: true, ..Default::default() });
    // Caps Lock включён: «Shift+п» даёт «п», остальное — заглавные.
    let mut keys = strokes("привет ", Lang::Ru);
    for (i, k) in keys.iter_mut().enumerate() {
        if i < 6 {
            k.caps = true;
            k.shift = i == 0;
        }
    }
    let mut fixes = Vec::new();
    for k in keys {
        fixes.extend(e.on_key(k, Some(Lang::Ru)));
    }
    assert_eq!(fixes.len(), 1);
    assert_eq!((fixes[0].text.as_str(), fixes[0].caps_lock_off), ("Привет", true));
}

#[test]
fn typos_are_fixed_only_when_enabled() {
    assert!(late_fixes_for("собщение ", Lang::Ru).is_empty());
    let mut e = with_text(TextOptions { fix_typos: true, ..Default::default() });
    let fixes = type_text(&mut e, "собщение ", Lang::Ru);
    assert_eq!(fixes[0].text, "сообщение");
}

#[test]
fn sentence_start_is_capitalized() {
    let mut e = with_text(TextOptions { capitalize_sentences: true, ..Default::default() });
    // «Привет. как дела» — «как» после точки становится «Как»; первое слово не трогаем
    // (неизвестно, что было до него).
    let fixes: Vec<String> =
        type_text(&mut e, "привет. как дела ", Lang::Ru).into_iter().map(|f| f.text).collect();
    assert_eq!(fixes, ["Как"]);
}

#[test]
fn layout_fix_and_case_fix_combine() {
    let mut e = with_text(TextOptions { fix_case: true, ..Default::default() });
    let fixes = type_text(&mut e, "GHbdtn ", Lang::En);
    assert_eq!(fixes.len(), 1);
    assert_eq!((fixes[0].text.as_str(), fixes[0].switch_to), ("Привет", Some(Lang::Ru)));
}

#[test]
fn held_letter_is_not_a_word() {
    // В игре: зажали W (автоповтор), потом пробел — ничего не переводим.
    let mut e = Engine::new();
    let w = KeyInput { vk: 0x57, ..Default::default() };
    assert!(e.on_key(w, Some(Lang::En)).is_none());
    for _ in 0..20 {
        assert!(e.on_key(KeyInput { repeat: true, ..w }, Some(Lang::En)).is_none());
    }
    assert!(e.current_word().is_empty());
    assert!(e.on_key(KeyInput { vk: 0x20, ..Default::default() }, Some(Lang::En)).is_none());
}

#[test]
fn single_letter_follows_the_phrase() {
    // «press f» — мем, после английского слова одиночная f остаётся английской.
    assert!(fixes_for("press f ", Lang::En).is_empty());
    // Первое слово — по таблице: «b» → «и».
    assert_eq!(fixes_for("b ", Lang::En), ["и"]);
    // В русской фразе (раскладку не переключили, «и» набрано латиницей посреди русского)
    // одиночная буква исправляется.
    let mut e = late();
    type_text(&mut e, "привет ", Lang::Ru);
    let fixes = type_text(&mut e, "b ", Lang::En);
    assert_eq!(fixes.len(), 1);
    assert_eq!(fixes[0].text, "и");
}

/// Клавиши английского слова — для прямой записи в обучение.
fn keys_of(text: &str) -> Vec<Stroke> {
    strokes(text, Lang::En).into_iter().map(|k| Stroke { vk: k.vk, shift: k.shift, caps: false }).collect()
}
