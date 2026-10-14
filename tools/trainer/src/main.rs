//! Строит языковые модели (`*.lm`) и словари словоформ (`*.fst`) для ядра.
//!
//!   bash tools/fetch-corpora.sh
//!   cargo run --release -p trainer -- [--order 5] [--prune 8] [--min-freq 8]
//!
//! Результат пишется в `crates/engine/data/` и коммитится: для сборки программы корпуса не нужны.

use fst::MapBuilder;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

struct Options {
    order: usize,
    prune: u32,
    min_freq: u64,
    corpora: PathBuf,
    out: PathBuf,
}

fn parse_args() -> Options {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut o = Options {
        order: 5,
        prune: 8,
        min_freq: 8,
        corpora: root.join("corpora"),
        out: root.join("crates/engine/data"),
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| panic!("{a}: нужно значение"));
        match a.as_str() {
            "--order" => o.order = val().parse().expect("--order"),
            "--prune" => o.prune = val().parse().expect("--prune"),
            "--min-freq" => o.min_freq = val().parse().expect("--min-freq"),
            "--corpora" => o.corpora = val().into(),
            "--out" => o.out = val().into(),
            _ => panic!("неизвестный аргумент {a}"),
        }
    }
    o
}

/// Частотный класс слова: ⌊log2(частота)⌋. Хранится в словаре и задаёт силу словарного бонуса.
fn freq_class(freq: u64) -> u64 {
    63 - freq.max(1).leading_zeros() as u64
}

/// Нормализованное слово, если оно целиком из алфавита языка.
fn normalize(word: &str, alphabet: &str) -> Option<String> {
    let w = word.to_lowercase();
    let ok = !w.is_empty()
        && w.chars().all(|c| alphabet.contains(c))
        && !w.starts_with('\'')
        && !w.ends_with('\'');
    ok.then_some(w)
}

/// Читает частотный список «слово частота» и складывает одинаковые после нормализации слова.
fn read_freq(path: &Path, alphabet: &str, min_freq: u64) -> BTreeMap<String, u64> {
    let file = fs::File::open(path).unwrap_or_else(|e| {
        panic!("{}: {e}\nСначала скачайте корпуса: bash tools/fetch-corpora.sh", path.display())
    });
    let mut out = BTreeMap::new();
    for line in BufReader::new(file).lines() {
        let line = line.expect("чтение корпуса");
        let mut parts = line.split_whitespace();
        let (Some(word), Some(freq)) = (parts.next(), parts.next()) else { continue };
        let Ok(freq) = freq.parse::<u64>() else { continue };
        if freq < min_freq {
            continue;
        }
        if let Some(w) = normalize(word, alphabet) {
            *out.entry(w).or_default() += freq;
        }
    }
    out
}

fn read_extra(path: &Path, alphabet: &str) -> Vec<String> {
    let text = fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .flat_map(str::split_whitespace)
        .filter_map(|w| normalize(w, alphabet))
        .collect()
}

fn build(lang: &str, freq_file: &str, extra: Option<&str>, o: &Options) {
    let alphabet = langmodel::alphabet(lang);
    let mut words = read_freq(&o.corpora.join(freq_file), alphabet, o.min_freq);
    eprintln!("{lang}: {} словоформ с частотой ≥ {}", words.len(), o.min_freq);

    // Вес слова в модели растёт логарифмически: иначе «и» и «в» задавили бы всё остальное.
    // Вес 1 у самых редких слов нужен оценке скидок Kneser-Ney (n1, n2).
    let weighted: Vec<(&str, u32)> =
        words.iter().map(|(w, &f)| (w.as_str(), freq_class(f).max(1) as u32)).collect();
    let tables = langmodel::train::train_pruned(lang, o.order, o.prune, weighted.into_iter());
    for n in 1..=o.order {
        eprintln!(
            "  порядок {n}: {} n-грамм, {} контекстов, D = {:.3}",
            tables.grams[n].len(),
            tables.contexts.get(n).map_or(0, Vec::len),
            tables.discount[n]
        );
    }
    let lm = tables.to_bytes();
    let lm_path = o.out.join(format!("{lang}.lm"));
    fs::write(&lm_path, &lm).expect("запись модели");
    eprintln!("  {} — {} КБ", lm_path.display(), lm.len() / 1024);

    // Добавочные слова получают средний частотный класс, если их нет в корпусе.
    if let Some(extra) = extra {
        for w in read_extra(&Path::new(env!("CARGO_MANIFEST_DIR")).join(extra), alphabet) {
            words.entry(w).or_insert(1 << 12);
        }
    }
    let mut builder = MapBuilder::memory();
    for (w, &f) in &words {
        builder.insert(w, freq_class(f)).expect("слова отсортированы");
    }
    let dict = builder.into_inner().expect("сборка FST");
    let dict_path = o.out.join(format!("{lang}.fst"));
    fs::write(&dict_path, &dict).expect("запись словаря");
    eprintln!("  {} — {} слов, {} КБ", dict_path.display(), words.len(), dict.len() / 1024);
}

fn main() {
    let o = parse_args();
    fs::create_dir_all(&o.out).expect("каталог для моделей");
    build("ru", "ru_freq.txt", Some("data/ru_extra.txt"), &o);
    build("en", "en_freq.txt", Some("data/en_extra.txt"), &o);
}
