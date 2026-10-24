//! Конвейер принятия решения по завершённому слову. Проверки идут по порядку,
//! первая сработавшая завершает анализ.

use crate::context::PhraseContext;
use crate::keys::{render_word, Stroke};
use crate::lexicon;
use crate::score::{score, Params, ScoreParts};
use layouts::Lang;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Вмешиваться только при явном перевесе.
    Careful,
    Normal,
    /// Переключать даже при небольшом перевесе.
    Aggressive,
}

impl Mode {
    /// Коэффициенты режима: режимы отличаются только порогом.
    pub fn params(self) -> Params {
        let normal = Params::default();
        let margin = match self {
            Mode::Careful => normal.margin * 1.75,
            Mode::Normal => normal.margin,
            Mode::Aggressive => normal.margin * 0.5,
        };
        Params { margin, ..normal }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Технический токен: путь, почта, идентификатор, аббревиатура, смесь с цифрами.
    Protected,
    TooShort,
    /// Однобуквенное или двухбуквенное служебное слово.
    ShortWord,
    /// Набрано обычное словарное слово — оставляем как есть.
    KnownWord,
    /// Решение по баллам.
    Score,
    /// Выучено на действиях пользователя (Pause).
    Learned,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// `Some(язык)`, если слово надо перевести в этот язык.
    pub switch_to: Option<Lang>,
    pub reason: Reason,
    pub typed: ScoreParts,
    pub other: ScoreParts,
    /// Перевес другого варианта с учётом контекста (в натах); решение — `delta >= margin`.
    pub delta: f32,
}

impl Decision {
    fn keep(reason: Reason) -> Self {
        Decision {
            switch_to: None,
            reason,
            typed: ScoreParts::default(),
            other: ScoreParts::default(),
            delta: 0.0,
        }
    }

    /// Решение по выученному предпочтению.
    pub fn learned(switch_to: Option<Lang>) -> Self {
        Decision { switch_to, ..Decision::keep(Reason::Learned) }
    }
}

/// Однобуквенные слова языков. Двухбуквенные проверяются по частотным словарям
/// (см. [`short_word`]).
const ONE_LETTER_RU: &[&str] = &["и", "с", "в", "к", "я", "о", "у", "а"];
const ONE_LETTER_EN: &[&str] = &["a", "i"];

/// Русские сокращения через точку: в английской раскладке точка — это «/».
const RU_DOT_ABBR: &[&str] = &["т.е.", "т.к.", "т.д.", "т.п.", "т.н.", "т.ч.", "н.э."];

/// Обычная пунктуация по краям слова (в обеих раскладках она не буква).
pub const ORDINARY_EDGE: &str = ",.!?:;\"'()-";

/// Двухбуквенное слово переводится, только если в другом языке оно очень частое
/// (частотный класс ≥ 14, т.е. ≥ 16 тыс. употреблений в корпусе) и намного частее набранного:
/// разрыв ≥ 6 классов (в 64 раза). «bp»→«из» (11→18) переводится, «ем»→«tv» (11→16) — нет.
const SHORT_MIN_CLASS: u8 = 14;
const SHORT_CLASS_GAP: u8 = 6;

pub fn decide(keys: &[Stroke], typed: Lang, ctx: &PhraseContext, p: &Params) -> Decision {
    let typed_text = render_word(keys, typed);
    let other = typed.other();
    let other_text = render_word(keys, other);

    // «т.е.», «т.к.» в английской раскладке — `n/t/`, `n/r/`: из-за «/» похоже на путь.
    let dotted = other_text.to_lowercase();
    if other == Lang::Ru && RU_DOT_ABBR.contains(&dotted.trim_end_matches([',', ':', ';', '!', '?'])) {
        return Decision { switch_to: Some(other), ..Decision::keep(Reason::ShortWord) };
    }

    if is_protected(&typed_text) {
        return Decision::keep(Reason::Protected);
    }

    if let Some(target) = short_word(&typed_text, typed, &other_text, ctx) {
        return Decision { switch_to: Some(target), ..Decision::keep(Reason::ShortWord) };
    }

    // Длину считаем по более «буквенному» варианту: «Это» в английской раскладке — это `"nj`,
    // где кавычка — край, а букв всего две.
    let letters = |s: &str| s.chars().filter(|c| c.is_alphabetic()).count();
    // Набранное без единой буквы (многоточие, знаки) — не слово, даже если в другой
    // раскладке это буквы: «...» не должно стать «ююю».
    if letters(&typed_text) == 0 || letters(&typed_text).max(letters(&other_text)) < 3 {
        return Decision::keep(Reason::TooShort);
    }

    let typed_parts = score(&typed_text, typed, p);
    let other_parts = score(&other_text, other, p);
    if typed_parts.class.is_some_and(|c| c >= p.known_class) && !typed_parts.odd_edges {
        return Decision { typed: typed_parts, other: other_parts, ..Decision::keep(Reason::KnownWord) };
    }
    // Набранного нет в словаре даже среди редких слов, а в другой раскладке — известное слово.
    // Так ловятся сокращения из одних согласных, которые модель знает плохо: `vaw` → «мфц».
    if typed_parts.class.is_none()
        && other_parts.class.is_some_and(|c| c >= p.known_class)
        && !other_parts.odd_edges
    {
        return Decision {
            switch_to: Some(other),
            typed: typed_parts,
            other: other_parts,
            ..Decision::keep(Reason::KnownWord)
        };
    }
    let delta = (other_parts.total() + ctx.bias(other, p)) - (typed_parts.total() + ctx.bias(typed, p));
    // Апостроф между буквами английского слова (`rock'n'roll`, `y'all`) модель оценивает плохо.
    // Если все куски между апострофами — частые английские слова, это английский; иначе
    // апостроф — русская «э» (`c'rjyjvbnm` → «сэкономить»).
    let english_pieces = typed == Lang::En && {
        let core = core(&typed_text).to_lowercase();
        core.contains('\'')
            && core.split('\'').all(|piece| {
                piece.chars().count() == 1
                    || lexicon::freq_class(Lang::En, piece).is_some_and(|c| c >= p.known_class)
            })
    };
    let switch_to = (delta >= p.margin && !english_pieces).then_some(other);
    Decision { switch_to, reason: Reason::Score, typed: typed_parts, other: other_parts, delta }
}

/// Минимум букв для раннего переключения посреди слова.
const EARLY_MIN_LETTERS: usize = 4;
/// Набранное начало слова не должно продолжаться ни в одно слово хотя бы такой частоты…
const EARLY_TYPED_CLASS: u8 = 8;
/// …а другое — должно продолжаться в частое слово.
const EARLY_OTHER_CLASS: u8 = 10;

/// Раннее переключение: можно ли уверенно исправить недописанное слово прямо сейчас.
/// Условия строже, чем на границе слова: ошибка посреди набора заметнее.
pub fn decide_early(keys: &[Stroke], typed: Lang, ctx: &PhraseContext, p: &Params) -> Option<Lang> {
    let typed_text = render_word(keys, typed).to_lowercase();
    let other = typed.other();
    let other_text = render_word(keys, other).to_lowercase();
    if other == Lang::En && is_web_address(&other_text) && typed_text.chars().all(char::is_alphabetic) {
        return Some(other);
    }
    if other_text.chars().count() < EARLY_MIN_LETTERS
        || !other_text.chars().all(char::is_alphabetic)
        || !typed_text.chars().any(char::is_alphabetic)
        || is_protected(&typed_text)
    {
        return None;
    }
    // Набранное может оказаться началом настоящего слова — ждём конца.
    if typed_text.chars().all(char::is_alphabetic)
        && lexicon::has_word_with_prefix(typed, &typed_text, EARLY_TYPED_CLASS)
    {
        return None;
    }
    if !lexicon::has_word_with_prefix(other, &other_text, EARLY_OTHER_CLASS) {
        return None;
    }
    let lm = |text: &str, lang| {
        let core = text.trim_matches(|c: char| !c.is_alphanumeric());
        lexicon::model(lang).prefix_logprob(core)
    };
    let delta = (lm(&other_text, other) + ctx.bias(other, p)) - (lm(&typed_text, typed) + ctx.bias(typed, p));
    (delta >= p.margin + p.early_extra).then_some(other)
}

/// Можно ли перевести выученное слово посреди набора: в нём хотя бы две буквы и набранное
/// не начинает частое слово своего языка («ру» — клавиши `he` — ждёт: вдруг это «hello»).
pub fn learned_can_switch_early(keys: &[Stroke], typed: Lang) -> bool {
    let typed_text = render_word(keys, typed).to_lowercase();
    typed_text.chars().filter(|c| c.is_alphabetic()).count() >= 2
        && !(typed_text.chars().all(char::is_alphabetic)
            && lexicon::has_word_with_prefix(typed, &typed_text, EARLY_TYPED_CLASS))
}

/// Зоны доменов для раннего переключения. Без `.by`, `.kz`, `.su`: в русской раскладке
/// это `юин`, `юля`, `юыг` — концы обычных слов («июля»).
const WEB_ZONES: &[&str] = &["com", "ru", "org", "net", "io", "info", "dev", "ua"];

/// Начало адреса: `www.`, `http:`, `https:` или `имя.зона` с известной зоной.
fn is_web_address(text: &str) -> bool {
    if ["www.", "http:", "https:"].iter().any(|p| text.starts_with(p)) {
        return true;
    }
    let Some((host, zone)) = text.rsplit_once('.') else {
        return false;
    };
    WEB_ZONES.contains(&zone)
        && host.chars().count() >= 2
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// В каком языке набор `keys` правдоподобнее.
pub fn likely_lang(keys: &[Stroke]) -> Lang {
    let p = Params::default();
    let en = score(&render_word(keys, Lang::En), Lang::En, &p).total();
    let ru = score(&render_word(keys, Lang::Ru), Lang::Ru, &p).total();
    if ru > en {
        Lang::Ru
    } else {
        Lang::En
    }
}

/// Короткие служебные слова: для 1–2 букв модели не за что зацепиться. Сравниваются ядра
/// слов без краевых знаков: `"I`, набранное в русской раскладке, — это `ЭШ`.
fn short_word(typed_text: &str, typed: Lang, other_text: &str, ctx: &PhraseContext) -> Option<Lang> {
    let t = core(typed_text).to_lowercase();
    let o = core(other_text).to_lowercase();
    let other = typed.other();
    let one_letter = |lang| match lang {
        Lang::Ru => ONE_LETTER_RU,
        Lang::En => ONE_LETTER_EN,
    };
    if o.is_empty() || !o.chars().all(char::is_alphabetic) || t.chars().count() > 2 {
        return None;
    }
    // Набранное само не должно быть словом: «a», «I», «и» не трогаем. Одиночные буквы
    // сверяем с таблицей (в субтитрах «b» и «d» частые), остальное — по словарю. Края
    // учитываем, только если они странные: `"да` — это «да», а `b[` — уже не «b».
    let typed_full = typed_text.to_lowercase();
    let ordinary_edges =
        typed_full.chars().filter(|c| !c.is_alphanumeric()).all(|c| ORDINARY_EDGE.contains(c));
    let looked_up = if ordinary_edges { &t } else { &typed_full };
    let typed_class = if looked_up.chars().count() == 1 {
        if one_letter(typed).contains(&looked_up.as_str()) {
            u8::MAX
        } else {
            0
        }
    } else {
        lexicon::freq_class(typed, looked_up).unwrap_or(0)
    };
    match o.chars().count() {
        // Одиночная буква — только если фраза не идёт на набранном языке: `press f` остаётся,
        // а `z [jxe` («я хочу») и «и» в русской фразе исправляются.
        1 => (one_letter(other).contains(&o.as_str())
            && typed_class < SHORT_MIN_CLASS
            && ctx.last_word() != Some(typed))
        .then_some(other),
        2 => {
            let other_class = lexicon::freq_class(other, &o)?;
            (other_class >= SHORT_MIN_CLASS && other_class >= typed_class.saturating_add(SHORT_CLASS_GAP))
                .then_some(other)
        }
        _ => None,
    }
}

/// Слово без знаков по краям.
fn core(text: &str) -> &str {
    text.trim_matches(|c: char| !c.is_alphanumeric())
}

/// Имена HTML-тегов: после `<` это разметка, а не русская «Б».
const HTML_TAGS: &[&str] = &[
    "a", "b", "i", "p", "br", "hr", "li", "ol", "ul", "td", "th", "tr", "em", "h1", "h2", "h3", "h4", "h5",
    "h6", "div", "span", "img", "table", "thead", "tbody", "form", "input", "button", "label", "select",
    "option", "script", "style", "link", "meta", "head", "body", "html", "title", "nav", "header", "footer",
    "section", "main", "pre", "code", "strong", "svg", "path", "template", "slot",
];

/// Технический текст, который нельзя трогать ни при каких баллах.
pub fn is_protected(text: &str) -> bool {
    // Буква диска: C:, D:\…
    let mut head = text.chars();
    if let (Some(drive), Some(':')) = (head.next(), head.next()) {
        if drive.is_ascii_alphabetic() {
            return true;
        }
    }
    // Начало HTML-тега без «>»: `<td class=…`. По краю `<` — это «Б», но `Бев`, `Бек` — не слова.
    if text.strip_prefix('<').is_some_and(|name| HTML_TAGS.contains(&name.to_ascii_lowercase().as_str())) {
        return true;
    }
    let core = text.trim_matches(|c: char| !c.is_alphanumeric());
    if core.is_empty() {
        return false;
    }
    let has_digit = core.chars().any(|c| c.is_ascii_digit());

    // Цифры: win32, x86, 127.0.0.1, 3.14. Слово, набранное не в той раскладке, цифр не содержит.
    if has_digit {
        return true;
    }
    // Пути, почта, идентификаторы, присваивания, host:port, URL.
    // Технические знаки — в любом месте токена. А `< > { } ~` по краям — это русские
    // заглавные Б Ю Х Ъ Ё (`<elm` = «Будь»), поэтому они защищают только внутри слова
    // или парой (`<stdio.h>`). Двоеточие — тоже только внутри, иначе попало бы любое «Note:».
    // То же с `@ # $ ^ &`: по краям это русские `" № ; : ?` («чего?» = `xtuj&`).
    // Одиночные буквы через `/` — не путь, а русские точки: `z/` = «я.», `n/t/` = «т.е.».
    let dotted = text.contains('/')
        && text.trim_end_matches('/').split('/').all(|s| {
            let mut c = s.chars();
            c.next().is_some_and(char::is_alphabetic) && c.next().is_none()
        });
    if (!dotted && text.chars().any(|c| "_/\\=+|".contains(c)))
        || core.chars().any(|c| "@#$%^&*:<>{}~".contains(c))
        || (text.contains('<') && text.contains('>'))
    {
        return true;
    }
    // Сокращения из одиночных букв через точку: p.m., e.g., i.e.
    if core.contains('.') && core.split('.').all(|s| s.chars().count() == 1) {
        return true;
    }
    // Смесь кириллицы и латиницы.
    if core.chars().any(is_cyrillic) && core.chars().any(|c| c.is_ascii_alphabetic()) {
        return true;
    }
    let letters: Vec<char> = core.chars().filter(|c| c.is_alphabetic()).collect();
    // Аббревиатуры из 2–4 заглавных: API, HTTP, SQL.
    if (2..=4).contains(&letters.len()) && letters.iter().all(|c| c.is_uppercase()) {
        return true;
    }
    // camelCase: строчная, за которой идёт заглавная. Заглавные Э Б Ю Х Ъ Ж Ё по краям —
    // это знаки другой раскладки (`"Literally"` = `ЭДшеукфдднЭ`), а не горб camelCase.
    let chars: Vec<char> = core.trim_matches(|c| "ЭБЮХЪЖЁ".contains(c)).chars().collect();
    if chars.windows(2).any(|w| w[0].is_lowercase() && w[1].is_uppercase()) {
        return true;
    }
    false
}

fn is_cyrillic(c: char) -> bool {
    matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_tokens() {
        for t in ["win32", "user_name", "a=b", "host:port", "user@mail", "C:", "API", "camelCase", "D:\\dev"]
        {
            assert!(is_protected(t), "{t} должен быть защищён");
        }
        for t in ["hello", "ghbdtn", "Привет", "nfr;t", "Hello,"] {
            assert!(!is_protected(t), "{t} не должен быть защищён");
        }
    }
}
