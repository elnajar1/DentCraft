//! Interface translations, built on Project Fluent (https://projectfluent.org).
//!
//! English (`locales/en.ftl`) is the source language and the fallback: a message that is
//! missing from another language, or that fails to format there, is shown in English, and a
//! message missing from English shows its id. Translation stays in the presentation layer:
//! case files, internal identifiers, user-entered names and geometry never pass through here.
//!
//! The `.ftl` files are compiled into the binary, so every run sees the same text.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use dentcraft_case::{CaseIssue, Material, RestorationKind, ScanRole};
use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use unic_langid::LanguageIdentifier;

/// A language of the interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    English,
    Spanish,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::English, Self::Spanish];

    /// Value stored in the app's persistent storage and taken by `--lang`.
    pub fn key(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Spanish => "es",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.key() == key)
    }

    /// The language's name in that language, as language menus show it.
    pub fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Spanish => "Español",
        }
    }

    /// Two-letter code for the compact selector.
    pub fn short(self) -> &'static str {
        match self {
            Self::English => "EN",
            Self::Spanish => "ES",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Self::English => include_str!("../locales/en.ftl"),
            Self::Spanish => include_str!("../locales/es.ftl"),
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

type Bundle = FluentBundle<FluentResource>;

static CURRENT: AtomicU8 = AtomicU8::new(0);
static BUNDLES: [OnceLock<Bundle>; 2] = [OnceLock::new(), OnceLock::new()];

/// Sets the language the interface is drawn in from the next frame on.
pub fn set_language(language: Language) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}

pub fn language() -> Language {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Language::Spanish,
        _ => Language::English,
    }
}

/// The message `id` in the current language.
pub fn message(id: &str) -> String {
    translate(language(), id, None)
}

/// The message `id` in the current language, with its variables filled from `args`.
pub fn message_args(id: &str, args: &FluentArgs) -> String {
    translate(language(), id, Some(args))
}

/// The message `id` in `language`, falling back to English and then to the id itself.
pub fn translate(language: Language, id: &str, args: Option<&FluentArgs>) -> String {
    let english = bundle(Language::English);
    if language == Language::English {
        return format(english, id, args, false).unwrap_or_else(|| id.to_owned());
    }
    format(bundle(language), id, args, true)
        .or_else(|| format(english, id, args, false))
        .unwrap_or_else(|| id.to_owned())
}

/// `tr!("id")` or `tr!("id", count = n, tooth = t.to_string())`: the message in the current
/// language. Variable names are Rust identifiers, so `.ftl` files spell them in snake_case.
macro_rules! tr {
    ($id:expr) => {
        $crate::i18n::message($id)
    };
    ($id:expr, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = ::fluent_bundle::FluentArgs::new();
        $(args.set(stringify!($name), $value);)+
        $crate::i18n::message_args($id, &args)
    }};
}
pub(crate) use tr;

/// Shown name of a restoration type. The case keeps its own (English) label.
pub fn restoration(kind: RestorationKind) -> String {
    message(match kind {
        RestorationKind::Crown => "restoration-crown",
        RestorationKind::Coping => "restoration-coping",
        RestorationKind::Inlay => "restoration-inlay",
        RestorationKind::Onlay => "restoration-onlay",
        RestorationKind::Veneer => "restoration-veneer",
        RestorationKind::Pontic => "restoration-pontic",
    })
}

/// Shown name of a scan role.
pub fn scan_role(role: ScanRole) -> String {
    message(match role {
        ScanRole::Upper => "role-upper",
        ScanRole::Lower => "role-lower",
        ScanRole::UpperPreOp => "role-upper-pre-op",
        ScanRole::LowerPreOp => "role-lower-pre-op",
        ScanRole::Bite => "role-bite",
        ScanRole::Other => "role-other",
    })
}

/// Shown name of a material: a built-in preset's translated name, or the name stored in the
/// case for anything else. The stored name itself never changes.
pub fn material_name(material: &Material) -> String {
    let presets = [
        (Material::zirconia(), "material-zirconia"),
        (
            Material::lithium_disilicate(),
            "material-lithium-disilicate",
        ),
        (Material::pmma(), "material-pmma"),
        (Material::wax(), "material-wax"),
    ];
    presets
        .into_iter()
        .find(|(preset, _)| preset.name == material.name)
        .map_or_else(|| material.name.clone(), |(_, id)| message(id))
}

/// A case issue as a sentence for the case setup screen.
pub fn case_issue(issue: &CaseIssue) -> String {
    match issue {
        CaseIssue::DuplicateTooth(t) => tr!("issue-duplicate-tooth", tooth = t.to_string()),
        CaseIssue::ConnectorNotAdjacent(a, b) => {
            tr!(
                "issue-connector-not-adjacent",
                a = a.to_string(),
                b = b.to_string()
            )
        }
        CaseIssue::ConnectorWithoutUnits(a, b) => {
            tr!(
                "issue-connector-without-units",
                a = a.to_string(),
                b = b.to_string()
            )
        }
        CaseIssue::FloatingPontic(t) => tr!("issue-floating-pontic", tooth = t.to_string()),
        CaseIssue::MissingJawScan(arch, t) => {
            tr!(
                "issue-missing-jaw-scan",
                arch = *arch,
                tooth = t.to_string()
            )
        }
    }
}

fn bundle(language: Language) -> &'static Bundle {
    BUNDLES[language.index()].get_or_init(|| build(language.key(), language.source()))
}

/// A bundle for `source`. Syntax errors drop only the broken entries; tests keep the shipped
/// files free of them.
fn build(tag: &str, source: &str) -> Bundle {
    let id: LanguageIdentifier = tag.parse().unwrap_or_default();
    let mut bundle = Bundle::new_concurrent(vec![id]);
    // Unicode isolation marks around placeables render as boxes in egui's fonts; right-to-left
    // layout is a separate task.
    bundle.set_use_isolating(false);
    let resource = FluentResource::try_new(source.to_owned()).unwrap_or_else(|(r, _)| r);
    // Duplicate ids keep their first definition; tests reject them.
    let _ = bundle.add_resource(resource);
    bundle
}

/// Formats `id`, or `None` when it is missing; when `strict`, a formatting error is `None` too,
/// so the caller can fall back to English.
fn format(bundle: &Bundle, id: &str, args: Option<&FluentArgs>, strict: bool) -> Option<String> {
    let pattern = bundle.get_message(id)?.value()?;
    let mut errors = Vec::new();
    let text = bundle.format_pattern(pattern, args, &mut errors);
    (!strict || errors.is_empty()).then(|| text.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluent_bundle::FluentValue;

    /// Message ids of an `.ftl` source: entries start at the first column as `id =`.
    fn ids(source: &str) -> Vec<String> {
        assert!(
            FluentResource::try_new(source.to_owned()).is_ok(),
            "syntax error"
        );
        source
            .lines()
            .filter(|l| l.starts_with(|c: char| c.is_ascii_lowercase()))
            .filter_map(|l| l.split_once(" =").map(|(id, _)| id.trim().to_owned()))
            .collect()
    }

    fn args(pairs: &[(&'static str, FluentValue<'static>)]) -> FluentArgs<'static> {
        let mut a = FluentArgs::new();
        for (k, v) in pairs {
            a.set(*k, v.clone());
        }
        a
    }

    #[test]
    fn shipped_files_parse_without_errors_or_duplicates() {
        for language in Language::ALL {
            let all = ids(language.source());
            let mut unique = all.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(all.len(), unique.len(), "{language:?} has duplicate ids");
            let bundle = build(language.key(), language.source());
            for id in &all {
                assert!(bundle.has_message(id), "{language:?}: {id}");
            }
        }
    }

    #[test]
    fn translations_only_use_english_ids() {
        let english = ids(Language::English.source());
        for language in Language::ALL {
            for id in ids(language.source()) {
                assert!(
                    english.contains(&id),
                    "{language:?} has `{id}`, English does not"
                );
            }
        }
    }

    #[test]
    fn every_message_formats_in_every_language() {
        // Variables some messages need; a missing one is a formatting error.
        let a = args(&[
            ("count", 3.into()),
            ("tooth", "16".into()),
            ("a", "14".into()),
            ("b", "15".into()),
            ("quadrant", 1.into()),
            ("tooth_type", "molar".into()),
            ("arch", "upper".into()),
            ("kind", "Crown".into()),
        ]);
        for id in ids(Language::English.source()) {
            for language in Language::ALL {
                let text = translate(language, &id, Some(&a));
                assert!(!text.trim().is_empty() && text != id, "{language:?}: {id}");
            }
        }
    }

    #[test]
    fn missing_messages_fall_back_to_english_then_to_the_id() {
        let english = build("en", "only-english = Only English\nboth = Both\n");
        let spanish = build("es", "both = Ambos\nbroken = { $missing }\n");
        let pick = |id: &str| {
            format(&spanish, id, None, true)
                .or_else(|| format(&english, id, None, false))
                .unwrap_or_else(|| id.to_owned())
        };
        assert_eq!(pick("both"), "Ambos");
        assert_eq!(pick("only-english"), "Only English");
        assert_eq!(pick("nowhere"), "nowhere");
        // A message that fails to format in Spanish is not shown half-filled.
        assert_eq!(format(&spanish, "broken", None, true), None);

        assert_eq!(
            translate(Language::Spanish, "no-such-id", None),
            "no-such-id"
        );
    }

    #[test]
    fn plurals_follow_each_language() {
        let units = |language, n: u32| {
            translate(language, "teeth-units", Some(&args(&[("count", n.into())])))
        };
        assert_eq!(units(Language::English, 1), "1 unit");
        assert_eq!(units(Language::English, 2), "2 units");
        assert_eq!(units(Language::Spanish, 1), "1 unidad");
        assert_eq!(units(Language::Spanish, 2), "2 unidades");
        assert_eq!(units(Language::Spanish, 0), "0 unidades");
    }

    #[test]
    fn tooth_descriptions_follow_each_language_word_order() {
        let describe = |language, quadrant: u8, kind: &'static str| {
            let a = args(&[("quadrant", quadrant.into()), ("tooth_type", kind.into())]);
            translate(language, "tooth-description", Some(&a))
        };
        assert_eq!(describe(Language::English, 1, "molar"), "Upper right molar");
        assert_eq!(
            describe(Language::English, 3, "canine"),
            "Lower left canine"
        );
        assert_eq!(
            describe(Language::Spanish, 1, "molar"),
            "Molar superior derecho"
        );
        assert_eq!(
            describe(Language::Spanish, 4, "premolar"),
            "Premolar inferior derecho"
        );
    }

    #[test]
    fn placeables_carry_no_isolation_marks() {
        let text = translate(
            Language::Spanish,
            "tooth-menu-title",
            Some(&args(&[("tooth", "16".into())])),
        );
        assert_eq!(text, "Diente 16");
        assert!(!text.contains(['\u{2068}', '\u{2069}']));
    }

    #[test]
    fn language_keys_round_trip() {
        for language in Language::ALL {
            assert_eq!(Language::from_key(language.key()), Some(language));
        }
        assert_eq!(Language::from_key("xx"), None);
        assert_eq!(Language::default(), Language::English);
    }
}
