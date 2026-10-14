//! Проверка качества распознавания: «печатает» тысячи реальных фраз физическими клавишами
//! и считает, сколько слов испорчено (ложные срабатывания) и сколько не исправлено (пропуски).
//!
//!   cargo run --release -p eval -- [--limit 2000] [--examples 10] [--sweep]
//!                                  [--margin 6] [--dict-floor 4] [--dict-slope 0.3] [--edge -3] [--ctx 0.75]
//!
//! Фразы — из Tatoeba (bash tools/fetch-corpora.sh), технический текст — tools/eval/data/tech.txt.
//! Модели обучены на другом корпусе (OpenSubtitles), так что проверка честная.

use engine::{Engine, KeyInput, Lang, Params, Tail};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

const VK_RETURN: u16 = 0x0D;

/// `--fix-case`: проверять и исправление регистра (по умолчанию выключено, чтобы мерить только раскладку).
static FIX_CASE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct Options {
    limit: usize,
    examples: usize,
    sweep: bool,
    explain: Vec<String>,
    params: Params,
    corpora: PathBuf,
}

fn parse_args() -> Options {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut o = Options {
        limit: 2000,
        examples: 0,
        sweep: false,
        explain: Vec::new(),
        params: Params::default(),
        corpora: root.join("corpora"),
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--fix-case" {
            FIX_CASE.store(true, std::sync::atomic::Ordering::Relaxed);
            continue;
        }
        if a == "--sweep" {
            o.sweep = true;
            continue;
        }
        let v = args.next().unwrap_or_else(|| panic!("{a}: нужно значение"));
        let f = || v.parse::<f32>().unwrap_or_else(|_| panic!("{a}: число"));
        match a.as_str() {
            "--explain" => o.explain.push(v),
            "--limit" => o.limit = v.parse().expect("--limit"),
            "--examples" => o.examples = v.parse().expect("--examples"),
            "--margin" => o.params.margin = f(),
            "--dict-floor" => o.params.dict_floor = f(),
            "--known" => o.params.known_class = v.parse().expect("--known"),
            "--dict-slope" => o.params.dict_slope = f(),
            "--edge" => o.params.edge_logp = f(),
            "--ctx" => o.params.context_weight = f(),
            "--corpora" => o.corpora = v.into(),
            _ => panic!("неизвестный аргумент {a}"),
        }
    }
    o
}

/// Типографские знаки, которых нет на клавиатуре, заменяются тем, что человек набрал бы на самом деле.
fn keyboard_text(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{2019}' | '\u{2018}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '«' | '»' => '"',
            '\u{2014}' | '\u{2013}' => '-',
            '\u{A0}' => ' ',
            c => c,
        })
        .collect::<String>()
        .replace('\u{2026}', "...")
}

/// Фразы Tatoeba на языке: каждая k-я, чтобы выборка была детерминированной и разнообразной.
/// Фразы с буквами, которых нет на клавиатуре (í, é, ß), пропускаются: их набирают особыми способами.
fn load_sentences(path: &Path, lang: Lang, limit: usize) -> Vec<String> {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}\nСначала: bash tools/fetch-corpora.sh", path.display()));
    let typeable = |s: &String| s.chars().all(|c| !c.is_alphabetic() || layouts::locate(c, lang).is_some());
    let all: Vec<String> =
        text.lines().filter_map(|l| l.splitn(3, '\t').nth(2)).map(keyboard_text).filter(typeable).collect();
    let step = (all.len() / limit.max(1)).max(1);
    all.into_iter().step_by(step).take(limit).collect()
}

/// Разбор решения по слову: как оно выглядит в обеих раскладках и из чего сложились баллы.
fn explain(word: &str, params: &Params) {
    let lower = word.to_lowercase();
    println!(
        "{word}: класс EN {:?}, RU {:?}",
        engine::freq_class(Lang::En, &lower),
        engine::freq_class(Lang::Ru, &lower)
    );
    for typed in [Lang::En, Lang::Ru] {
        let keys: Option<Vec<engine::Stroke>> = word
            .chars()
            .map(|c| layouts::locate(c, typed).map(|(vk, shift)| engine::Stroke { vk, shift, caps: false }))
            .collect();
        let Some(keys) = keys else { continue };
        let d = engine::detect::decide(&keys, typed, &engine::PhraseContext::default(), params);
        let other = engine::keys::render_word(&keys, typed.other());
        println!(
            "{word} ({typed}) ↔ {other} ({}): {:?}, перевес {:.2} при пороге {:.1}",
            typed.other(),
            d.switch_to,
            d.delta,
            params.margin
        );
        println!("    набрано: {:?}\n    другое:  {:?}", d.typed, d.other);
    }
}

/// Симуляция окна: что окажется на экране, если человек печатает `text` пальцами,
/// привыкшими к раскладке `intended`, а окно на самом деле в раскладке `layout`.
fn simulate(engine: &mut Engine, text: &str, intended: Lang, mut layout: Lang) -> String {
    let mut screen: Vec<char> = Vec::new();
    let keys =
        text.chars().map(|c| (c, layouts::locate(c, intended))).chain([('\n', Some((VK_RETURN, false)))]);
    for (c, key) in keys {
        let Some((vk, shift)) = key else {
            // Символ не с клавиатуры (тире, кавычки-ёлочки): вставлен как-то иначе, буфер сбрасывается.
            engine.reset();
            screen.push(c);
            continue;
        };
        match engine.on_key(KeyInput { vk, shift, ..Default::default() }, Some(layout)) {
            Some(fix) => {
                screen.truncate(screen.len().saturating_sub(fix.erase));
                screen.extend(fix.text.chars());
                match fix.tail {
                    Tail::Char(t) => screen.push(t),
                    Tail::Key(VK_RETURN) => screen.push('\n'),
                    Tail::Key(_) => screen.push('\t'),
                    Tail::None => {}
                }
                layout = fix.switch_to.unwrap_or(layout);
            }
            None if vk == VK_RETURN => screen.push('\n'),
            None => screen.extend(layouts::render(vk, shift, false, layout)),
        }
    }
    screen.into_iter().collect::<String>().trim_end_matches('\n').to_string()
}

/// Число слов, которые отличаются от ожидаемых.
fn word_errors(got: &str, want: &str) -> usize {
    let (g, w): (Vec<_>, Vec<_>) = (got.split_whitespace().collect(), want.split_whitespace().collect());
    g.iter().zip(&w).filter(|(a, b)| a != b).count() + g.len().abs_diff(w.len())
}

#[derive(Default)]
struct Stat {
    words: usize,
    errors: usize,
    examples: Vec<String>,
}

impl Stat {
    fn add(&mut self, words: usize, errors: usize, example: impl FnOnce() -> String, keep: usize) {
        self.words += words;
        self.errors += errors;
        if errors > 0 && self.examples.len() < keep {
            self.examples.push(example());
        }
    }

    fn rate(&self) -> f64 {
        100.0 * self.errors as f64 / self.words.max(1) as f64
    }
}

#[derive(Default)]
struct Report {
    /// Фраза набрана в правильной раскладке — любое изменение лишнее.
    fp: [Stat; 2],
    /// Фраза набрана в неправильной раскладке — должна исправиться целиком.
    miss: [Stat; 2],
    /// Отдельные слова в неправильной раскладке, без контекста.
    word_miss: [Stat; 2],
    /// Отдельные слова в правильной раскладке, без контекста.
    word_fp: [Stat; 2],
    tech_fp: Stat,
}

fn idx(lang: Lang) -> usize {
    match lang {
        Lang::Ru => 0,
        Lang::En => 1,
    }
}

fn run(sets: &[(Lang, Vec<String>)], tech: &[String], params: Params, keep: usize) -> Report {
    let mut r = Report::default();
    let fresh = || {
        let mut e = Engine::new();
        e.params = params;
        e.text.fix_case = FIX_CASE.load(std::sync::atomic::Ordering::Relaxed);
        e
    };
    for (lang, sentences) in sets {
        let (lang, i) = (*lang, idx(*lang));
        for s in sentences {
            let n = s.split_whitespace().count();
            let ok = simulate(&mut fresh(), s, lang, lang);
            r.fp[i].add(n, word_errors(&ok, s), || format!("{s}\n      → {ok}"), keep);
            let fixed = simulate(&mut fresh(), s, lang, lang.other());
            r.miss[i].add(n, word_errors(&fixed, s), || format!("{s}\n      → {fixed}"), keep);
            for w in s.split_whitespace() {
                let got = simulate(&mut fresh(), w, lang, lang);
                r.word_fp[i].add(1, (got != w) as usize, || format!("{w} → {got}"), keep);
                let got = simulate(&mut fresh(), w, lang, lang.other());
                r.word_miss[i].add(1, (got != w) as usize, || format!("{w} → {got}"), keep);
            }
        }
    }
    for line in tech {
        let got = simulate(&mut fresh(), line, Lang::En, Lang::En);
        let n = line.split_whitespace().count();
        r.tech_fp.add(n, word_errors(&got, line), || format!("{line}\n      → {got}"), keep);
    }
    r
}

fn print_report(r: &Report, keep: usize) {
    let row = |name: &str, s: &[Stat; 2]| {
        println!("{name:<44} {:>7.2}%  {:>7.2}%", s[0].rate(), s[1].rate());
    };
    println!("{:<44} {:>8}  {:>8}", "доля испорченных слов", "RU", "EN");
    row("ложные: фраза в правильной раскладке", &r.fp);
    row("пропуски: фраза в неправильной раскладке", &r.miss);
    row("ложные: отдельное слово, без контекста", &r.word_fp);
    row("пропуски: отдельное слово, без контекста", &r.word_miss);
    println!("{:<44} {:>7.2}%  ({} слов)", "ложные: технический текст", r.tech_fp.rate(), r.tech_fp.words);
    if keep == 0 {
        return;
    }
    let mut out = String::new();
    let mut section = |title: &str, s: &Stat| {
        if !s.examples.is_empty() {
            let _ = writeln!(out, "\n## {title}");
            for e in &s.examples {
                let _ = writeln!(out, "  {e}");
            }
        }
    };
    for (lang, i) in [("RU", 0), ("EN", 1)] {
        section(&format!("{lang}: ложные во фразах"), &r.fp[i]);
        section(&format!("{lang}: пропуски во фразах"), &r.miss[i]);
        section(&format!("{lang}: ложные, отдельные слова"), &r.word_fp[i]);
        section(&format!("{lang}: пропуски, отдельные слова"), &r.word_miss[i]);
    }
    section("технический текст", &r.tech_fp);
    print!("{out}");
}

fn main() {
    let o = parse_args();
    if !o.explain.is_empty() {
        for w in &o.explain {
            explain(w, &o.params);
        }
        return;
    }
    let sets = vec![
        (Lang::Ru, load_sentences(&o.corpora.join("rus_sentences.tsv"), Lang::Ru, o.limit)),
        (Lang::En, load_sentences(&o.corpora.join("eng_sentences.tsv"), Lang::En, o.limit)),
    ];
    let tech: Vec<String> = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("data/tech.txt"))
        .expect("data/tech.txt")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect();
    engine::warm_up();

    if o.sweep {
        println!(
            "{:>6} | {:>7} {:>7} | {:>7} {:>7} | {:>7} {:>7} | {:>6}",
            "порог", "лжRU", "лжEN", "прRU", "прEN", "слRU", "слEN", "тех"
        );
        for m in [2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0, 12.0] {
            let r = run(&sets, &tech, Params { margin: m, ..o.params }, 0);
            println!(
                "{m:>6.1} | {:>6.2}% {:>6.2}% | {:>6.2}% {:>6.2}% | {:>6.2}% {:>6.2}% | {:>5.2}%",
                r.fp[0].rate(),
                r.fp[1].rate(),
                r.miss[0].rate(),
                r.miss[1].rate(),
                r.word_miss[0].rate(),
                r.word_miss[1].rate(),
                r.tech_fp.rate()
            );
        }
        return;
    }
    println!("{:?}\n", o.params);
    let r = run(&sets, &tech, o.params, o.examples);
    print_report(&r, o.examples);
}
