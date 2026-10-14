//! Макросы в тексте сокращений: `{date}`, `{time}`, `{clipboard}`, `{cursor}`.

/// Текущие дата и время: (день, месяц, год, часы, минуты).
pub type Now = (u32, u32, u32, u32, u32);

/// Раскрывает макросы. Возвращает текст и на сколько символов вернуть курсор назад
/// (до места `{cursor}`). Буфер обмена читается, только если он нужен.
pub fn expand(text: &str, now: Now, clipboard: impl FnOnce() -> Option<String>) -> (String, usize) {
    let (day, month, year, hour, minute) = now;
    let mut out = text
        .replace("{date}", &format!("{day:02}.{month:02}.{year}"))
        .replace("{time}", &format!("{hour:02}:{minute:02}"));
    if out.contains("{clipboard}") {
        out = out.replace("{clipboard}", &clipboard().unwrap_or_default());
    }
    match out.find("{cursor}") {
        Some(pos) => {
            let after = out[pos + "{cursor}".len()..].replace("{cursor}", "");
            let back = after.chars().filter(|&c| c != '\r').count();
            (format!("{}{after}", &out[..pos]), back)
        }
        None => (out, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Now = (7, 3, 2026, 9, 5);

    #[test]
    fn cursor_macro() {
        assert_eq!(expand("Здравствуйте, {cursor}!", NOW, || None), ("Здравствуйте, !".to_string(), 1));
        assert_eq!(expand("без курсора", NOW, || None), ("без курсора".to_string(), 0));
    }

    #[test]
    fn date_time_and_clipboard() {
        assert_eq!(expand("{date} {time}", NOW, || None).0, "07.03.2026 09:05");
        assert_eq!(expand("[{clipboard}]", NOW, || Some("буфер".into())).0, "[буфер]");
    }
}
