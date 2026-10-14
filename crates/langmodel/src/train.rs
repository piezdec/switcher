//! Обучение модели по взвешенному списку слов. Используется `tools/trainer` и тестами.

use crate::{alphabet, key, Tables, BITS, BOUNDARY, MAX_ORDER};
use std::collections::{HashMap, HashSet};

/// `words` — пары (слово в нижнем регистре, вес). Слова с символами вне алфавита пропускаются.
/// `prune` — n-граммы старшего порядка с весом меньше этого порога отбрасываются.
pub fn train<'a>(lang: &str, order: usize, words: impl Iterator<Item = (&'a str, u32)>) -> Tables {
    train_pruned(lang, order, 0, words)
}

pub fn train_pruned<'a>(
    lang: &str,
    order: usize,
    prune: u32,
    words: impl Iterator<Item = (&'a str, u32)>,
) -> Tables {
    assert!((1..=MAX_ORDER).contains(&order));
    let alphabet: Vec<char> = alphabet(lang).chars().collect();
    let index: HashMap<char, u8> = alphabet.iter().enumerate().map(|(i, &c)| (c, i as u8 + 1)).collect();

    // Сырые счётчики по порядкам.
    let mut raw: Vec<HashMap<u32, u64>> = vec![HashMap::new(); order + 1];
    'words: for (word, weight) in words {
        let mut seq = vec![BOUNDARY];
        for c in word.chars() {
            match index.get(&c) {
                Some(&s) => seq.push(s),
                None => continue 'words,
            }
        }
        if seq.len() == 1 {
            continue;
        }
        seq.push(BOUNDARY);
        for (n, table) in raw.iter_mut().enumerate().skip(1) {
            for w in seq.windows(n) {
                *table.entry(key(w)).or_default() += weight as u64;
            }
        }
    }

    // Kneser-Ney: младшие порядки считают не частоту n-граммы, а у скольких разных символов
    // она бывает продолжением. Исключение — n-граммы от границы слова: перед ними ничего нет.
    let mut grams: Vec<Vec<(u32, u32)>> = vec![Vec::new(); order + 1];
    for n in 1..=order {
        let mut v: Vec<(u32, u32)> = if n == order {
            raw[n]
                .iter()
                .filter(|&(_, &c)| c >= prune as u64)
                .map(|(&k, &c)| (k, c.min(u32::MAX as u64) as u32))
                .collect()
        } else {
            let mut cont: HashMap<u32, HashSet<u8>> = HashMap::new();
            for &k in raw[n + 1].keys() {
                let first = (k >> (BITS * n as u32)) as u8;
                let suffix = k & ((1u32 << (BITS * n as u32)) - 1);
                cont.entry(suffix).or_default().insert(first);
            }
            raw[n]
                .iter()
                .map(|(&k, &c)| {
                    let starts_at_boundary = (k >> (BITS * (n as u32 - 1))) as u8 == BOUNDARY;
                    let value = if starts_at_boundary {
                        c.min(u32::MAX as u64) as u32
                    } else {
                        cont.get(&k).map_or(0, |s| s.len() as u32)
                    };
                    (k, value)
                })
                .filter(|&(_, c)| c > 0)
                .collect()
        };
        v.sort_unstable();
        grams[n] = v;
    }

    // Скидки: D = n1 / (n1 + 2·n2) для каждого порядка.
    // Для старшего порядка — по счётчикам до отсечения, иначе n1 всегда ноль.
    let mut discount = vec![0.0f32; order + 1];
    for n in 1..=order {
        let counts: Vec<u64> = if n == order {
            raw[n].values().copied().collect()
        } else {
            grams[n].iter().map(|e| e.1 as u64).collect()
        };
        let n1 = counts.iter().filter(|&&c| c == 1).count() as f32;
        let n2 = counts.iter().filter(|&&c| c == 2).count() as f32;
        discount[n] = if n1 + n2 > 0.0 { (n1 / (n1 + 2.0 * n2)).clamp(0.1, 0.95) } else { 0.5 };
    }

    // Контексты: сумма и число продолжений — из тех же таблиц, иначе вероятности не сложатся в 1.
    let mut contexts: Vec<Vec<(u32, u32, u32)>> = vec![Vec::new(); order + 1];
    for n in 2..=order {
        let mut acc: HashMap<u32, (u64, u32)> = HashMap::new();
        for &(k, c) in &grams[n] {
            let e = acc.entry(k >> BITS).or_default();
            e.0 += c as u64;
            e.1 += 1;
        }
        let mut v: Vec<(u32, u32, u32)> =
            acc.into_iter().map(|(k, (t, ty))| (k, t.min(u32::MAX as u64) as u32, ty)).collect();
        v.sort_unstable();
        contexts[n] = v;
    }

    Tables { order, alphabet, discount, grams, contexts }
}
