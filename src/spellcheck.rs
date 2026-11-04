//! Spell checking for the composer with Hunspell dictionaries.
//!
//! Dictionaries come from the places desktop systems install them, plus a
//! `dictionaries` directory in the application's data directory for
//! platforms without system dictionaries. Several languages can be checked
//! at once: a word is correct when any chosen dictionary accepts it, so a
//! message may mix, for example, Dutch and English. Words the user adds go
//! into a personal list that every language accepts.
//!
//! Loading a dictionary can take a moment, so it happens on a worker thread.
//! Until it finishes nothing is marked. Every change to what is checked
//! bumps [`epoch`], which the composer includes in its layout key to draw
//! the marks again.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// The most suggestions a context menu offers for one word.
const MAX_SUGGESTIONS: usize = 6;
/// Checked words remembered between keystrokes, before the cache starts over.
const MAX_CACHED_WORDS: usize = 20_000;

struct Speller {
    dictionaries: Vec<spellbook::Dictionary>,
    personal: HashSet<String>,
    cache: Mutex<HashMap<String, bool>>,
}

impl Speller {
    fn correct(&self, word: &str) -> bool {
        if self.personal.contains(word) || self.personal.contains(&word.to_lowercase()) {
            return true;
        }
        if let Some(known) = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(word)
        {
            return *known;
        }
        let correct = self
            .dictionaries
            .iter()
            .any(|dictionary| dictionary.check(word))
            // A hyphenated compound is fine when each part is.
            || (word.contains('-')
                && word.split('-').filter(|part| !part.is_empty()).all(|part| {
                    self.dictionaries.iter().any(|dictionary| dictionary.check(part))
                }));
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.len() >= MAX_CACHED_WORDS {
            cache.clear();
        }
        cache.insert(word.to_owned(), correct);
        correct
    }
}

fn speller() -> &'static RwLock<Option<Arc<Speller>>> {
    static SPELLER: OnceLock<RwLock<Option<Arc<Speller>>>> = OnceLock::new();
    SPELLER.get_or_init(Default::default)
}

static EPOCH: AtomicU64 = AtomicU64::new(0);
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Also look for dictionaries in `<data_dir>/dictionaries`.
pub(crate) fn set_data_dir(data_dir: &Path) {
    let _ = DATA_DIR.set(data_dir.to_owned());
}
/// The newest configuration; an older load that finishes later is dropped.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Changes whenever the marks may change: dictionaries loaded or switched
/// off, or a word added.
pub(crate) fn epoch() -> u64 {
    EPOCH.load(Ordering::Acquire)
}

fn current() -> Option<Arc<Speller>> {
    speller()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// Check with these dictionaries, by code such as "nl" or "en_US", and accept
/// the personal words. No languages switches checking off.
pub(crate) fn configure(languages: &[String], personal: &[String]) {
    let data_dir = DATA_DIR.get().map(PathBuf::as_path);
    let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let personal = personal
        .iter()
        .map(|word| word.trim().to_owned())
        .filter(|word| !word.is_empty())
        .collect::<HashSet<_>>();
    if languages.is_empty() {
        *speller()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        EPOCH.fetch_add(1, Ordering::AcqRel);
        return;
    }
    let available = available_in(&search_dirs(data_dir));
    let files = languages
        .iter()
        .filter_map(|language| available.get(language).cloned())
        .collect::<Vec<_>>();
    std::thread::Builder::new()
        .name("spellcheck-load".into())
        .spawn(move || {
            let dictionaries = files
                .iter()
                .filter_map(|(aff, dic)| match load(aff, dic) {
                    Ok(dictionary) => Some(dictionary),
                    Err(error) => {
                        tracing::warn!(%error, path = %dic.display(), "could not load dictionary");
                        None
                    }
                })
                .collect::<Vec<_>>();
            if GENERATION.load(Ordering::Acquire) != generation {
                return;
            }
            *speller()
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(Speller {
                dictionaries,
                personal,
                cache: Mutex::default(),
            }));
            EPOCH.fetch_add(1, Ordering::AcqRel);
        })
        .ok();
}

/// Accept `word` from now on, as the personal list will after saving.
pub(crate) fn add_personal_word(word: &str) {
    let Some(current) = current() else {
        return;
    };
    let mut personal = current.personal.clone();
    personal.insert(word.to_owned());
    // Dictionaries are shared, not copied: they are immutable once loaded.
    let next = Speller {
        dictionaries: Vec::new(),
        personal,
        cache: Mutex::default(),
    };
    let mut slot = speller()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(existing) = slot.take() {
        match Arc::try_unwrap(existing) {
            Ok(mut owned) => {
                owned.personal = next.personal;
                owned.cache = Mutex::default();
                *slot = Some(Arc::new(owned));
            }
            Err(shared) => {
                // Another thread still reads it; it keeps its copy and the
                // word applies from the next configuration.
                *slot = Some(shared);
            }
        }
    }
    drop(slot);
    EPOCH.fetch_add(1, Ordering::AcqRel);
}

fn load(aff: &Path, dic: &Path) -> Result<spellbook::Dictionary, String> {
    let aff = read_dictionary_text(aff)?;
    let dic = read_dictionary_text(dic)?;
    spellbook::Dictionary::new(&aff, &dic).map_err(|error| error.to_string())
}

/// Hunspell files name their encoding in the affix file's `SET` line. UTF-8
/// is read as is and ISO 8859-1/-15 byte for byte; other encodings would
/// misread words, so those dictionaries are skipped.
fn read_dictionary_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    match String::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(error) => {
            let bytes = error.into_bytes();
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_uppercase();
            if head.contains("ISO8859-1") || head.contains("ISO8859-15") || !head.contains("SET ") {
                Ok(bytes.iter().map(|byte| char::from(*byte)).collect())
            } else {
                Err("unsupported dictionary encoding".into())
            }
        }
    }
}

fn search_dirs(data_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(data_dir) = data_dir {
        dirs.push(data_dir.join("dictionaries"));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/share/hunspell"));
        dirs.push(home.join("Library/Spelling"));
    }
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        dirs.push(data_home.join("hunspell"));
    }
    for dir in [
        "/usr/share/hunspell",
        "/usr/share/myspell/dicts",
        "/usr/share/myspell",
        "/usr/local/share/hunspell",
        "/app/share/hunspell",
        "/run/host/usr/share/hunspell",
        "/Library/Spelling",
    ] {
        dirs.push(PathBuf::from(dir));
    }
    dirs
}

/// Dictionaries found, by code, with their affix and word files. The first
/// directory that has a language wins.
fn available_in(dirs: &[PathBuf]) -> HashMap<String, (PathBuf, PathBuf)> {
    let mut found = HashMap::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("dic") {
                continue;
            }
            let Some(code) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            // Hyphenation and thesaurus files share the directory.
            if code.starts_with("hyph_") || code.starts_with("th_") {
                continue;
            }
            let aff = path.with_extension("aff");
            if aff.is_file() && !found.contains_key(code) {
                found.insert(code.to_owned(), (aff, path.clone()));
            }
        }
    }
    found
}

/// The dictionaries that can be chosen, by code, sorted.
pub(crate) fn available_languages() -> Vec<String> {
    let mut codes = available_in(&search_dirs(DATA_DIR.get().map(PathBuf::as_path)))
        .into_keys()
        .collect::<Vec<_>>();
    codes.sort();
    codes
}

/// The languages to check when the user has not chosen any: the interface
/// or system language, and English.
pub(crate) fn default_languages(locale: &str, available: &[String]) -> Vec<String> {
    let language = locale
        .split(['-', '_', '.', '@'])
        .next()
        .unwrap_or_default()
        .to_lowercase();
    let region = locale
        .split(['-', '_', '.', '@'])
        .nth(1)
        .unwrap_or_default()
        .to_uppercase();
    let mut chosen: Vec<String> = Vec::new();
    let pick = |chosen: &mut Vec<String>, candidates: &[String]| {
        if let Some(code) = candidates.iter().find(|code| available.contains(code))
            && !chosen.contains(code)
        {
            chosen.push(code.clone());
        }
    };
    if !language.is_empty() {
        pick(
            &mut chosen,
            &[
                format!("{language}_{region}"),
                language.clone(),
                format!("{language}_{}", language.to_uppercase()),
            ],
        );
    }
    if !chosen
        .iter()
        .any(|code| code == "en" || code.starts_with("en_"))
    {
        pick(&mut chosen, &["en_US".into(), "en_GB".into(), "en".into()]);
    }
    chosen
}

/// A readable name for a dictionary code.
pub(crate) fn language_name(code: &str) -> String {
    let name = match code {
        "nl" | "nl_NL" => "Nederlands",
        "nl_BE" => "Nederlands (België)",
        "en_US" => "English (US)",
        "en_GB" => "English (UK)",
        "en_AU" => "English (Australia)",
        "en_CA" => "English (Canada)",
        "de_DE" | "de" => "Deutsch",
        "de_AT" => "Deutsch (Österreich)",
        "de_CH" => "Deutsch (Schweiz)",
        "fr_FR" | "fr" => "Français",
        "fr_BE" => "Français (Belgique)",
        "es_ES" | "es" => "Español",
        "it_IT" | "it" => "Italiano",
        "pt_PT" => "Português",
        "pt_BR" => "Português (Brasil)",
        "da_DK" | "da" => "Dansk",
        "sv_SE" | "sv" => "Svenska",
        "nb_NO" => "Norsk bokmål",
        "pl_PL" | "pl" => "Polski",
        "ru_RU" | "ru" => "Русский",
        "tr_TR" | "tr" => "Türkçe",
        _ => return code.to_owned(),
    };
    format!("{name} · {code}")
}

/// The languages to check with these settings: none when checking is off,
/// the chosen ones, or else the interface language with English.
pub(crate) fn effective_languages(
    enabled: bool,
    chosen: &[String],
    interface_language: &str,
    available: &[String],
) -> Vec<String> {
    if !enabled {
        return Vec::new();
    }
    if chosen.is_empty() {
        default_languages(interface_language, available)
    } else {
        chosen
            .iter()
            .filter(|code| available.contains(code))
            .cloned()
            .collect()
    }
}

/// The misspelled words in `text`, as byte ranges. The word holding
/// `caret` is left alone while it is being typed. Quoted lines, addresses
/// and links are not checked.
pub(crate) fn misspelled(text: &str, caret: Option<usize>) -> Vec<Range<usize>> {
    let Some(speller) = current().filter(|speller| !speller.dictionaries.is_empty()) else {
        return Vec::new();
    };
    words(text)
        .filter(|range| caret.is_none_or(|caret| caret < range.start || caret > range.end))
        .filter(|range| !speller.correct(&text[range.clone()]))
        .collect()
}

/// The word at `offset` when it is misspelled, with suggestions for it.
pub(crate) fn lookup(text: &str, offset: usize) -> Option<(Range<usize>, Vec<String>)> {
    let speller = current().filter(|speller| !speller.dictionaries.is_empty())?;
    let range = words(text).find(|range| range.start <= offset && offset <= range.end)?;
    let word = &text[range.clone()];
    if speller.correct(word) {
        return None;
    }
    let mut suggestions = Vec::new();
    for dictionary in &speller.dictionaries {
        let mut found = Vec::new();
        dictionary.suggest(word, &mut found);
        for suggestion in found {
            if !suggestions.contains(&suggestion) {
                suggestions.push(suggestion);
            }
        }
        if suggestions.len() >= MAX_SUGGESTIONS {
            break;
        }
    }
    suggestions.truncate(MAX_SUGGESTIONS);
    Some((range, suggestions))
}

/// The checkable words of `text`: letters with inner apostrophes, hyphens
/// and dots (as in "e-mail" or "z.s.m."), outside quoted lines, links and
/// addresses, and without digits.
fn words(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut ranges = Vec::new();
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let quoted = line.trim_start().starts_with('>');
        if !quoted {
            let mut offset = 0;
            for chunk in line.split(char::is_whitespace) {
                let start = line_start + offset;
                offset += chunk.len()
                    + line[offset + chunk.len()..]
                        .chars()
                        .next()
                        .map_or(0, char::len_utf8);
                if chunk.is_empty() || looks_like_address(chunk) {
                    continue;
                }
                word_ranges(chunk, start, &mut ranges);
            }
        }
        line_start += line.len();
    }
    ranges.into_iter()
}

fn looks_like_address(chunk: &str) -> bool {
    chunk.contains('@')
        || chunk.contains("://")
        || chunk.starts_with("www.")
        || chunk.contains('/')
        || chunk.contains('\\')
        // A domain such as example.com, but not an abbreviation such as z.s.m.
        || chunk
            .split('.')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .windows(2)
            .any(|parts| parts[0].len() > 1 && parts[1].len() > 1 && chunk.contains('.'))
}

fn word_ranges(chunk: &str, base: usize, ranges: &mut Vec<Range<usize>>) {
    let joiner = |c: char| matches!(c, '\'' | '’' | '-');
    let mut start: Option<usize> = None;
    let mut end = 0;
    let mut has_digit = false;
    let mut flush = |start: &mut Option<usize>, end: usize, has_digit: &mut bool| {
        if let Some(begin) = start.take() {
            let word = &chunk[begin..end];
            let word = word.trim_end_matches(joiner);
            if word.chars().count() > 1 && !*has_digit {
                ranges.push(base + begin..base + begin + word.len());
            }
        }
        *has_digit = false;
    };
    for (index, character) in chunk.char_indices() {
        if character.is_alphabetic() {
            if start.is_none() {
                start = Some(index);
            }
            end = index + character.len_utf8();
        } else if character.is_ascii_digit() {
            if start.is_some() {
                has_digit = true;
                end = index + 1;
            }
        } else if joiner(character) && start.is_some() {
            end = index + character.len_utf8();
        } else {
            flush(&mut start, end, &mut has_digit);
        }
    }
    flush(&mut start, end, &mut has_digit);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words_of(text: &str) -> Vec<&str> {
        words(text).map(|range| &text[range]).collect()
    }

    #[test]
    fn words_skip_quotes_links_addresses_and_numbers() {
        let text = "Hallo Jan, zie https://example.com of mail ton@example.nl.\n> geciteerd foutje\nOm 10u, de e-mail van z'n broer.";
        assert_eq!(
            words_of(text),
            [
                "Hallo", "Jan", "zie", "of", "mail", "Om", "de", "e-mail", "van", "z'n", "broer"
            ]
        );
    }

    #[test]
    fn several_dictionaries_accept_a_word_any_of_them_knows() {
        let english = spellbook::Dictionary::new("SET UTF-8\n", "2\nhello\nworld\n").unwrap();
        let dutch = spellbook::Dictionary::new("SET UTF-8\n", "2\nhallo\nwereld\n").unwrap();
        let speller = Speller {
            dictionaries: vec![english, dutch],
            personal: HashSet::from(["Flectar".to_owned()]),
            cache: Mutex::default(),
        };
        assert!(speller.correct("hello"));
        assert!(speller.correct("wereld"));
        assert!(speller.correct("Flectar"));
        assert!(speller.correct("hallo-world"));
        assert!(!speller.correct("helo"));
    }

    /// With the system's Dutch and English Hunspell dictionaries, when present.
    #[test]
    fn mixed_dutch_and_english_text_marks_only_real_mistakes() {
        let dir = Path::new("/usr/share/hunspell");
        let (Ok(nl), Ok(en)) = (
            load(&dir.join("nl.aff"), &dir.join("nl.dic")),
            load(&dir.join("en_US.aff"), &dir.join("en_US.dic")),
        ) else {
            return;
        };
        let speller = Speller {
            dictionaries: vec![nl, en],
            personal: HashSet::new(),
            cache: Mutex::default(),
        };
        let text = "Hallo Jan, bedankt voor je e-mail. The meeting is tomorow en ik kom morgn.";
        let wrong = words(text)
            .filter(|range| !speller.correct(&text[range.clone()]))
            .map(|range| &text[range])
            .collect::<Vec<_>>();
        assert_eq!(wrong, ["tomorow", "morgn"]);
    }

    #[test]
    fn default_languages_follow_the_locale_and_add_english() {
        let available = ["de_DE", "en_GB", "en_US", "nl", "nl_BE"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            default_languages("nl_NL.UTF-8", &available),
            ["nl", "en_US"]
        );
        assert_eq!(default_languages("nl-BE", &available), ["nl_BE", "en_US"]);
        assert_eq!(default_languages("en_GB", &available), ["en_GB"]);
        assert_eq!(default_languages("fr_FR", &available), ["en_US"]);
    }
}
