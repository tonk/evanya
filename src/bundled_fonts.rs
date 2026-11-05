//! Fonts bundled so they can be chosen in Settings → Fonts on every
//! platform: SauceCodePro Nerd Font and GNU FreeFont. Their bytes live once
//! in the executable and are shared, not copied, with both font stacks:
//! Fontique for Slint's interface and fontdb for the composer's cosmic-text
//! editor.

use std::sync::Arc;

macro_rules! bundled {
    ($($path:literal),* $(,)?) => {
        [$(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/fonts/", $path)) as &[u8]),*]
    };
}

/// Each bundled font file.
pub(crate) const FONTS: [&[u8]; 16] = bundled![
    "sauce-code-pro-nerd-font/SauceCodeProNerdFont-Regular.ttf",
    "sauce-code-pro-nerd-font/SauceCodeProNerdFont-Bold.ttf",
    "sauce-code-pro-nerd-font/SauceCodeProNerdFont-Italic.ttf",
    "sauce-code-pro-nerd-font/SauceCodeProNerdFont-BoldItalic.ttf",
    "gnu-freefont/FreeSans.ttf",
    "gnu-freefont/FreeSansBold.ttf",
    "gnu-freefont/FreeSansOblique.ttf",
    "gnu-freefont/FreeSansBoldOblique.ttf",
    "gnu-freefont/FreeSerif.ttf",
    "gnu-freefont/FreeSerifBold.ttf",
    "gnu-freefont/FreeSerifItalic.ttf",
    "gnu-freefont/FreeSerifBoldItalic.ttf",
    "gnu-freefont/FreeMono.ttf",
    "gnu-freefont/FreeMonoBold.ttf",
    "gnu-freefont/FreeMonoOblique.ttf",
    "gnu-freefont/FreeMonoBoldOblique.ttf",
];

/// Make the bundled fonts available to Slint, before the first window
/// lays out text.
pub(crate) fn register_for_interface() {
    use slint::fontique_011::fontique::Blob;
    let mut fonts = slint::fontique_011::shared_collection();
    for data in FONTS {
        fonts.register_fonts(Blob::new(Arc::new(data)), None);
    }
}

/// Make the bundled fonts available to the composer's editor.
pub(crate) fn register_for_editor(database: &mut cosmic_text::fontdb::Database) {
    for data in FONTS {
        database.load_font_source(cosmic_text::fontdb::Source::Binary(Arc::new(data)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_fonts_name_their_families() {
        let mut database = cosmic_text::fontdb::Database::new();
        register_for_editor(&mut database);
        let families = database
            .faces()
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
            .collect::<std::collections::HashSet<_>>();
        for family in [
            "SauceCodePro Nerd Font",
            "FreeSans",
            "FreeSerif",
            "FreeMono",
        ] {
            assert!(
                families.contains(family),
                "{family} is missing: {families:?}"
            );
        }
    }
}
