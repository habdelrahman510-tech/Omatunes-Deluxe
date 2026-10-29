use std::borrow::Cow;

use iced::widget::Text;
use iced::widget::text::{IntoFragment, Shaping};

pub fn text<'a>(fragment: impl IntoFragment<'a>) -> Text<'a> {
    Text::new(ltr_guard(fragment.into_fragment())).shaping(Shaping::Advanced)
}

fn contains_rtl(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c,
            '\u{0590}'..='\u{05FF}'
            | '\u{0600}'..='\u{06FF}'
            | '\u{0700}'..='\u{074F}'
            | '\u{0750}'..='\u{077F}'
            | '\u{0780}'..='\u{07BF}'
            | '\u{07C0}'..='\u{07FF}'
            | '\u{0800}'..='\u{08FF}'
            | '\u{FB1D}'..='\u{FDFF}'
            | '\u{FE70}'..='\u{FEFF}'
            | '\u{10800}'..='\u{10FFF}'
            | '\u{1E800}'..='\u{1EFFF}'
        )
    })
}

fn ltr_guard(fragment: Cow<'_, str>) -> Cow<'_, str> {
    if contains_rtl(&fragment) {
        Cow::Owned(format!("\u{200E}{fragment}"))
    } else {
        fragment
    }
}

/// Apply the LTR guard to every line of a multi-line string.
///
/// cosmic-text resolves each paragraph's base direction on its own, so a lone
/// leading LRM only fixes the first line. Long-form (unsynchronized) lyrics
/// arrive as one string, so each line has to be guarded individually —
/// otherwise every RTL line after the first still lays out right-aligned.
pub fn guard_lines(content: &str) -> Cow<'_, str> {
    if !contains_rtl(content) {
        return Cow::Borrowed(content);
    }

    let mut out = String::with_capacity(content.len() + 8);
    for (i, line) in content.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if contains_rtl(line) {
            out.push('\u{200E}');
        }
        out.push_str(line);
    }

    Cow::Owned(out)
}
