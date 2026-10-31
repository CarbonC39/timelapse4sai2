use super::i18n::Language;

/// Only used when no saved language exists. A manual choice takes precedence.
pub fn system_language() -> Language {
    #[cfg(target_os = "windows")]
    {
        // This reads the user's display language, rather than number/date formats.
        // SAFETY: GetUserDefaultUILanguage takes no arguments or pointers.
        let language_id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
        from_windows_language_id(language_id)
    }
    #[cfg(not(target_os = "windows"))]
    {
        from_unix_locales(["LC_ALL", "LC_MESSAGES", "LANG"].map(|name| std::env::var(name).ok()))
    }
}

#[cfg(any(target_os = "windows", test))]
fn from_windows_language_id(language_id: u16) -> Language {
    // The low ten bits contain the primary language; ignore regional variants.
    match language_id & 0x03ff {
        0x04 => Language::SimplifiedChinese,
        0x11 => Language::Japanese,
        _ => Language::English,
    }
}

#[cfg(any(not(target_os = "windows"), test))]
fn from_unix_locales(locales: [Option<String>; 3]) -> Language {
    // Respect the message locale's precedence, including C/POSIX overrides.
    let locale = locales
        .iter()
        .flatten()
        .find(|value| !value.trim().is_empty());
    let primary = locale
        .map(|value| {
            value
                .trim()
                .split(['_', '-', '.', '@'])
                .next()
                .unwrap_or("")
        })
        .unwrap_or("");
    if primary.eq_ignore_ascii_case("zh") {
        Language::SimplifiedChinese
    } else if primary.eq_ignore_ascii_case("ja") {
        Language::Japanese
    } else {
        Language::English
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_display_languages_match_supported_translations() {
        for id in [0x0804, 0x1004, 0x0404] {
            assert_eq!(from_windows_language_id(id), Language::SimplifiedChinese);
        }
        assert_eq!(from_windows_language_id(0x0411), Language::Japanese);
        for id in [0x0409, 0x0809, 0x0407, 0] {
            assert_eq!(from_windows_language_id(id), Language::English);
        }
    }

    #[test]
    fn unix_locales_handle_regions_encodings_and_fallbacks() {
        for locale in ["zh_CN.UTF-8", "zh-Hans-CN", "ZH_SG.utf8", "zh_TW"] {
            assert_eq!(
                from_unix_locales([None, None, Some(locale.into())]),
                Language::SimplifiedChinese
            );
        }
        assert_eq!(
            from_unix_locales([None, None, Some("ja_JP.UTF-8".into())]),
            Language::Japanese
        );
        for locale in ["en_US.UTF-8", "de_DE", "C", "POSIX", ""] {
            assert_eq!(
                from_unix_locales([None, None, Some(locale.into())]),
                Language::English
            );
        }
        assert_eq!(from_unix_locales([None, None, None]), Language::English);
    }

    #[test]
    fn unix_message_locale_respects_overrides() {
        assert_eq!(
            from_unix_locales([Some("C".into()), Some("ja_JP".into()), Some("zh_CN".into())]),
            Language::English
        );
        assert_eq!(
            from_unix_locales([Some(" ".into()), Some("ja_JP".into()), Some("zh_CN".into())]),
            Language::Japanese
        );
    }
}
