//! Ядро автопереключателя: не знает ничего про Windows.
//!
//! Платформенный слой скармливает [`Engine`] нажатия клавиш вместе с текущей раскладкой окна,
//! а в ответ иногда получает [`Fix`] — команду «стереть N символов и напечатать вот это».

mod context;
pub mod detect;
pub mod keys;
pub mod learning;
mod lexicon;
pub mod score;
pub mod text;
pub mod translit;

pub use context::PhraseContext;
pub use detect::{Decision, Mode, Reason};
pub use keys::Stroke;
pub use layouts::Lang;
pub use learning::Learning;
pub use lexicon::{freq_class, warm_up};
pub use score::Params;
pub use text::{Snippet, TextOptions};

use keys::{classify, render_word, KeyClass, VK_OEM_MINUS, VK_RETURN, VK_TAB};

/// Слова длиннее этого порога не анализируются: скорее всего это не слово, а мусор.
const MAX_WORD: usize = 48;

/// Клавиша `/` (в русской раскладке — точка).
const VK_OEM_2: u16 = 0xBF;

/// Слово — одиночные буквы через `/`: `n`, `n/t`. Продолжение `т.е.`, `т.к.`, а не путь.
fn dotted_letters(word: &[Stroke]) -> bool {
    word.len() % 2 == 1
        && word.iter().enumerate().all(|(i, s)| {
            if i % 2 == 0 {
                (0x41..=0x5A).contains(&s.vk)
            } else {
                s.vk == VK_OEM_2 && !s.shift
            }
        })
}

/// Нажатие клавиши вместе с состоянием модификаторов.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyInput {
    pub vk: u16,
    pub shift: bool,
    pub caps: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub win: bool,
    /// Автоповтор зажатой клавиши (а не новое нажатие).
    pub repeat: bool,
}

/// Что напечатать после исправленного слова вместо проглоченной клавиши-границы.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tail {
    #[default]
    None,
    /// Печатный символ уже в целевой раскладке (`?` на английской становится `,` на русской).
    Char(char),
    /// Непечатная клавиша, которую надо дослать как есть (Enter, Tab).
    Key(u16),
}

/// Команда платформенному слою: стереть `erase` символов, напечатать `text` и `tail`,
/// переключить окно на `switch_to` (если задано).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fix {
    pub erase: usize,
    pub text: String,
    pub tail: Tail,
    /// `None` — раскладку не трогать (сокращение, исправление регистра).
    pub switch_to: Option<Lang>,
    /// Текст — сокращение: раскрыть макросы `{date}`, `{clipboard}`…
    pub expand_macros: bool,
    /// Выключить Caps Lock (был включён случайно).
    pub caps_lock_off: bool,
    /// `None` — ручной перевод по горячей клавише или раннее переключение.
    pub decision: Option<Decision>,
}

/// Последнее завершённое слово — для ручного перевода и восстановления после Backspace.
#[derive(Clone, Debug)]
struct LastWord {
    keys: Vec<Stroke>,
    boundary: Stroke,
    /// В какой раскладке слово сейчас на экране.
    lang: Lang,
    /// Слово переключили мы (на границе или посреди набора): Pause по нему — это отмена.
    auto_fixed: bool,
}

#[derive(Debug)]
pub struct Engine {
    word: Vec<Stroke>,
    last: Option<LastWord>,
    context: PhraseContext,
    /// Слово начато сразу после дефиса, перед которым не было слова: `-f`, `--force`.
    /// Это флаг командной строки, его не трогаем.
    flag: bool,
    /// В текущем слове уже было раннее переключение.
    early_done: bool,
    /// В текущем слове был ручной перевод: язык и вес для обучения, учтётся на границе слова.
    manual: Option<(Lang, u8)>,
    /// Автоматическое переключение. Ручной перевод работает всегда.
    pub auto: bool,
    /// Коэффициенты распознавания (порог и пр.); пресеты — [`Mode::params`].
    pub params: Params,
    /// Выученные предпочтения пользователя.
    pub learning: Learning,
    /// Сокращения, регистр, опечатки.
    pub text: TextOptions,
    /// Следующее слово начинает предложение (для заглавной буквы).
    sentence_start: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Engine {
            word: Vec::new(),
            last: None,
            context: PhraseContext::default(),
            flag: false,
            early_done: false,
            manual: None,
            auto: true,
            params: Mode::Normal.params(),
            learning: Learning::default(),
            text: TextOptions::default(),
            sentence_start: false,
        }
    }
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.params = mode.params();
    }

    /// Обработка нажатия. `layout` — раскладка, в которой сейчас печатает окно
    /// (`None`, если это не одна из поддерживаемых раскладок или её не удалось узнать).
    ///
    /// Если вернулся `Some(Fix)`, платформа должна проглотить это нажатие и выполнить исправление.
    pub fn on_key(&mut self, k: KeyInput, layout: Option<Lang>) -> Option<Fix> {
        let mut class = classify(k.vk, k.shift);
        // На клавише `/` в русской раскладке точка. После одиночных букв через неё («т.е.» = `n/t/`)
        // она держит сокращение целиком, а не завершает слово: иначе вышли бы два слова «т» и «е».
        if class == KeyClass::Boundary && k.vk == VK_OEM_2 && !k.shift && dotted_letters(&self.word) {
            class = KeyClass::Word;
        }
        if k.ctrl || k.alt || k.win {
            // Сочетания клавиш: каретка или текст могли поменяться как угодно.
            if class != KeyClass::Modifier {
                self.reset();
            }
            return None;
        }
        let stroke = Stroke { vk: k.vk, shift: k.shift, caps: k.caps };
        match class {
            KeyClass::Modifier => None,
            KeyClass::Reset => {
                self.reset();
                None
            }
            KeyClass::Backspace => {
                self.backspace();
                None
            }
            // Зажатая буква с автоповтором — не слово: так бегают в играх (W, A, S, D), а не печатают.
            // Буфер сбрасываем, чтобы ни анализа, ни исправления не было.
            KeyClass::Word if k.repeat => {
                self.reset();
                None
            }
            KeyClass::Word => {
                if self.word.len() >= MAX_WORD {
                    self.reset();
                }
                self.word.push(stroke);
                self.try_early(layout)
            }
            KeyClass::Boundary => self.finish_word(stroke, layout),
        }
    }

    /// Раннее переключение посреди слова (не больше одного раза за слово). Текущее нажатие
    /// проглатывается, поэтому его символ входит в `text`.
    fn try_early(&mut self, layout: Option<Lang>) -> Option<Fix> {
        let typed = layout?;
        if !self.auto || !self.params.early || self.flag || self.early_done || self.manual.is_some() {
            return None;
        }
        // Пользователь просил такие слова не трогать.
        if self.learning.keeps_prefix(&self.word, typed) {
            return None;
        }
        let target = detect::decide_early(&self.word, typed, &self.context, &self.params)?;
        self.early_done = true;
        Some(Fix {
            erase: self.word.len() - 1,
            text: render_word(&self.word, target),
            tail: Tail::None,
            switch_to: Some(target),
            decision: None,
            ..Fix::default()
        })
    }

    fn finish_word(&mut self, boundary: Stroke, layout: Option<Lang>) -> Option<Fix> {
        let early = std::mem::take(&mut self.early_done);
        let manual = self.manual.take();
        let sentence_start = std::mem::take(&mut self.sentence_start);
        if self.word.is_empty() {
            // Две границы подряд: последнее слово больше не стоит вплотную к каретке.
            self.last = None;
            self.flag = boundary.vk == VK_OEM_MINUS;
            // «Конец. Начало»: пробел после точки не отменяет начало предложения.
            self.sentence_start = sentence_start || ends_sentence(boundary, layout);
            return None;
        }
        let keys = std::mem::take(&mut self.word);
        let flag = std::mem::take(&mut self.flag);

        // Сокращения — в любой раскладке и независимо от автопереключения.
        if let (Some(typed), None) = (layout, manual) {
            let renderings = [render_word(&keys, typed), render_word(&keys, typed.other())];
            if let Some(snippet) = self.text.snippet_for(&renderings) {
                let fix = Fix {
                    erase: keys.len(),
                    text: snippet.text.clone(),
                    tail: tail_for(boundary, typed),
                    expand_macros: true,
                    ..Fix::default()
                };
                // Раскрытый текст уже не совпадает с клавишами — переводить его по Pause нельзя.
                self.last = None;
                return Some(fix);
            }
        }

        let mut switch_to = None;
        let mut decision = None;
        let shown = match (layout, manual) {
            // Слово только что перевели вручную — запоминаем и больше не спорим.
            (Some(typed), Some((lang, weight))) => {
                self.learning.learn(&keys, lang, weight);
                typed
            }
            (Some(typed), None) if self.auto && !flag => {
                let d = self.decide(&keys, typed);
                switch_to = d.switch_to;
                decision = Some(d);
                switch_to.unwrap_or(typed)
            }
            (Some(typed), _) => typed,
            // Раскладка неизвестна. Важно это только для ручного перевода, а его зовут,
            // когда на экране неправдоподобный вариант.
            (None, _) => detect::likely_lang(&keys).other(),
        };

        // Текст слова, каким он будет на экране, и исправления поверх него.
        let original = render_word(&keys, shown);
        let mut text = original.clone();
        let mut caps_lock_off = false;
        let editable = layout.is_some() && manual.is_none() && !flag;
        // Регистр — до проверки на технический токен: `пРИВЕТ` похоже на camelCase, но у
        // исправления регистра свои строгие шаблоны по нажатиям.
        if editable && self.text.fix_case {
            if let Some((fixed, caps)) = text::fix_case(&keys, &text) {
                text = fixed;
                caps_lock_off = caps;
            }
        }
        if editable && !detect::is_protected(&text) {
            if self.text.fix_typos {
                if let Some(fixed) = text::fix_typo(&text, shown) {
                    text = fixed;
                }
            }
            if self.text.capitalize_sentences && sentence_start {
                if let Some(fixed) = text::capitalize(&text) {
                    text = fixed;
                }
            }
        }

        if layout.is_some() && original.chars().filter(|c| c.is_alphabetic()).count() >= 2 {
            self.context.push(shown);
        }
        // Было ли слово настоящим словом своего языка (для правила одиночных букв).
        let core = text.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        let known = lexicon::freq_class(shown, &core).is_some_and(|c| c >= self.params.known_class);
        self.context.set_last_word(known.then_some(shown));
        self.sentence_start = text.ends_with(['.', '!', '?']) || ends_sentence(boundary, Some(shown));
        let auto_fixed = switch_to.is_some() || (early && manual.is_none());
        let fix = (switch_to.is_some() || text != original).then(|| Fix {
            erase: keys.len(),
            text: text.clone(),
            tail: tail_for(boundary, shown),
            switch_to,
            caps_lock_off,
            decision,
            ..Fix::default()
        });
        // Tab в формах уводит фокус в другое поле: последнее слово там уже не стоит у каретки.
        // Слово с исправленным регистром или опечаткой Pause тоже не переводит: текст уже другой.
        let pausable = boundary.vk != VK_TAB && text == original;
        self.last = pausable.then_some(LastWord { keys, boundary, lang: shown, auto_fixed });
        fix
    }

    /// Решение по завершённому слову: сначала выученное, потом конвейер распознавания.
    fn decide(&self, keys: &[Stroke], typed: Lang) -> Decision {
        match self.learning.preference(keys) {
            Some(lang) => Decision::learned((lang != typed).then_some(lang)),
            None => detect::decide(keys, typed, &self.context, &self.params),
        }
    }

    /// Ручной перевод: текущего слова, а если оно пустое — последнего завершённого
    /// вместе с границей. Повторный вызов возвращает слово обратно. Каждый перевод учится.
    pub fn manual_convert(&mut self, layout: Option<Lang>) -> Option<Fix> {
        if !self.word.is_empty() {
            let from = layout.unwrap_or_else(|| detect::likely_lang(&self.word).other());
            let to = from.other();
            // Отмена раннего переключения весит как отмена; повторный Pause гасит предыдущий.
            self.manual = match self.manual {
                Some((lang, _)) if lang != to => None,
                _ if self.early_done => Some((to, learning::UNDO_WEIGHT)),
                _ => Some((to, learning::MANUAL_WEIGHT)),
            };
            return Some(Fix {
                erase: self.word.len(),
                text: render_word(&self.word, to),
                tail: Tail::None,
                switch_to: Some(to),
                decision: None,
                ..Fix::default()
            });
        }
        let last = self.last.as_mut()?;
        // После Enter команда уже могла выполниться — переписывать поздно.
        if last.boundary.vk == VK_RETURN {
            return None;
        }
        let to = last.lang.other();
        let boundary = last.boundary.render(to)?;
        let weight = if last.auto_fixed { learning::UNDO_WEIGHT } else { learning::MANUAL_WEIGHT };
        self.learning.learn(&last.keys, to, weight);
        // Повторный Pause — это уже возврат собственного перевода, а не отмена нашего.
        last.auto_fixed = false;
        let mut text = render_word(&last.keys, to);
        text.push(boundary);
        last.lang = to;
        Some(Fix { erase: last.keys.len() + 1, text, switch_to: Some(to), ..Fix::default() })
    }

    fn backspace(&mut self) {
        if self.word.pop().is_none() {
            // Стёрли границу — продолжаем редактировать предыдущее слово.
            if let Some(last) = self.last.take() {
                self.word = last.keys;
            }
        }
    }

    /// Каретка переместилась (клик, стрелки, сочетание клавиш): буфер больше не соответствует экрану.
    pub fn reset(&mut self) {
        self.sentence_start = false;
        self.word.clear();
        self.last = None;
        self.flag = false;
        self.early_done = false;
        self.manual = None;
    }

    /// Сменилось окно: сбрасываем и буфер, и контекст фразы.
    pub fn reset_context(&mut self) {
        self.reset();
        self.context.clear();
    }

    /// Текущее недописанное слово.
    pub fn current_word(&self) -> &[Stroke] {
        &self.word
    }
}

/// Граница слова, после которой начинается предложение: Enter или `. ! ?`.
fn ends_sentence(boundary: Stroke, layout: Option<Lang>) -> bool {
    boundary.vk == VK_RETURN
        || layout.and_then(|l| boundary.render(l)).is_some_and(|c| matches!(c, '.' | '!' | '?'))
}

fn tail_for(boundary: Stroke, target: Lang) -> Tail {
    match boundary.vk {
        VK_RETURN | VK_TAB => Tail::Key(boundary.vk),
        _ => boundary.render(target).map_or(Tail::Key(boundary.vk), Tail::Char),
    }
}

#[cfg(test)]
mod tests;
