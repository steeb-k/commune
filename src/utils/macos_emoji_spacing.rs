//! Emoji that do not run into each other, on macOS.
//!
//! Apple Color Emoji draws every glyph wider than the space it asks for. Its
//! advance is exactly the font size, and its inked area is exactly a quarter
//! wider than that, at every size — measured from 12 px through 48 px. So
//! consecutive emoji do not merely touch, they overlap by a quarter of their
//! width, and a line of coloured squares reads as one continuous bar.
//!
//! It is invisible on most emoji, which are round and carry their own visual
//! margin. It is glaring on the solid blocks that puzzle results are made of,
//! which is where it was noticed.
//!
//! **The font cannot simply be replaced.** That was the first plan and it does
//! not work: Pango on macOS uses `PangoCairoCoreTextFontMap`, so fontconfig is
//! not consulted at all, `CoreText` refuses Noto Color Emoji outright because
//! its colour tables are `CBDT` rather than `sbix` or `COLR`, and even the
//! `COLRv1` build — which does register — is never chosen, because
//! `libpangocairo` hard-codes the string `Apple Color Emoji` for the `emoji`
//! family. Nothing short of patching Pango changes which font is used.
//!
//! So the spacing is corrected instead. Pango adds letter spacing between
//! glyphs and not after the last one, so the visible gap between two emoji is
//! exactly what is added here minus the quarter they overlap by. A quarter is
//! therefore break-even — they stop overlapping and start touching — and
//! everything past that is the gap.

use gtk::{glib, pango, prelude::*};

/// How much to add between emoji, as a fraction of the font size.
///
/// A quarter of it is spent cancelling the overlap, so the gap this leaves is
/// `0.4 - 0.25`, or an eighth of an em — about 2.4 px against a 20 px glyph at
/// the default size. A third was tried first and leaves 1.3 px, which reads as
/// touching rather than separated.
const SPACING_EM: f64 = 0.4;

/// The resolution to assume if there are no settings to ask.
const FALLBACK_DPI: f64 = 96.0;

/// Whether this character is one the emoji font draws.
///
/// Deliberately only the pictographic planes, where nothing is ever text. The
/// arrows and dingbats below `U+2BFF` are left out: most of them render from
/// the body font, and spacing those would be a regression rather than a fix.
/// Joiners are included so that a sequence stays one run rather than being
/// broken into several.
fn is_emoji(c: char) -> bool {
    matches!(
        u32::from(c),
        0x1F000
            ..=0x1FAFF        // pictographs, emoticons, transport, symbols
        | 0xFE0F                 // the emoji variation selector
        | 0x200D // zero width joiner
    )
}

/// The font size of the given label, in pixels.
///
/// Read from the widget rather than from the settings, so that a label the
/// stylesheet has scaled — a Markdown heading, say — is spaced in proportion
/// to what it actually renders at.
fn font_size_px(label: &gtk::Label) -> Option<f64> {
    let context = label.pango_context();
    let description = context.font_description()?;
    let size = f64::from(description.size()) / f64::from(pango::SCALE);

    if size <= 0.0 {
        return None;
    }

    if description.is_size_absolute() {
        return Some(size);
    }

    // A point size has to be resolved against the same resolution Pango will
    // use, which on macOS is the one `super::macos_text_scale` set.
    let dpi = gtk::Settings::default()
        .map_or(FALLBACK_DPI, |settings| {
            f64::from(settings.gtk_xft_dpi()) / 1024.0
        })
        .max(1.0);

    Some(size * dpi / 72.0)
}

/// The letter spacing to apply to the emoji in the given text, if any.
fn attributes_for(text: &str, font_size_px: f64) -> Option<pango::AttrList> {
    let spacing = (font_size_px * SPACING_EM * f64::from(pango::SCALE)).round();
    let Ok(spacing) = i32::try_from(spacing as i64) else {
        return None;
    };
    if spacing <= 0 {
        return None;
    }

    let mut attributes: Option<pango::AttrList> = None;
    let mut run_start: Option<usize> = None;

    // Byte offsets, because that is what Pango indexes attributes by, and they
    // are into the label's displayed text rather than its markup.
    for (index, c) in text.char_indices().chain([(text.len(), '\0')]) {
        if is_emoji(c) {
            run_start.get_or_insert(index);
            continue;
        }

        let Some(start) = run_start.take() else {
            continue;
        };

        // A single emoji has nothing to overlap, but it can still run into
        // whatever follows it, so one is worth spacing too.
        let mut attribute = pango::AttrInt::new_letter_spacing(spacing);
        attribute.set_start_index(u32::try_from(start).unwrap_or(u32::MAX));
        attribute.set_end_index(u32::try_from(index).unwrap_or(u32::MAX));

        attributes
            .get_or_insert_with(pango::AttrList::new)
            .insert(attribute);
    }

    attributes
}

/// Apply the spacing this label's text needs, replacing whatever was there.
fn apply(label: &gtk::Label) {
    let attributes = font_size_px(label).and_then(|size| attributes_for(&label.text(), size));

    // Setting this to `None` is what clears it when the text stops having
    // emoji in it. `GtkLabel` merges these with whatever its markup set up, so
    // nothing else about the formatting is disturbed.
    label.set_attributes(attributes.as_ref());
}

/// Keep the emoji in the given label from overlapping, now and whenever its
/// text changes.
pub(crate) fn watch(label: &gtk::Label) {
    apply(label);

    label.connect_label_notify(apply);

    // The stylesheet is not applied until the label is in a window, so a
    // heading measured before that would be spaced for the default size.
    label.connect_map(apply);

    // And the whole scale moves if the text resolution does.
    if let Some(settings) = gtk::Settings::default() {
        settings.connect_gtk_xft_dpi_notify(glib::clone!(
            #[weak]
            label,
            move |_| apply(&label)
        ));
    }
}
