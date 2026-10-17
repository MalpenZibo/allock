//! Digits that roll to their next value, as on a mechanical counter. Every
//! digit is a column of the ten, clipped to one line and slid by a spring;
//! anything else in the text is drawn as it is.

use guido::prelude::*;

use super::spring;

#[derive(Clone, Copy)]
pub struct Look {
    pub size: f32,
    /// The clip, and so the distance between two digits.
    pub line: f32,
    pub weight: FontWeight,
    pub spacing: f32,
}

/// Tabular figures, so a rolling column is as wide whichever digit shows.
pub fn tabular() -> FontFeatures {
    FontFeatures::new().enable(b"tnum")
}

pub fn view(
    value: Memo<String>,
    look: Look,
    color: impl Fn() -> Color + Copy + 'static,
) -> Container {
    // Rebuilt only when the shape of the text changes ("9:59" to "10:00"),
    // so the columns of a ticking clock stay and roll.
    let pattern = create_memo(move || {
        value
            .get()
            .chars()
            .map(|c| if c.is_ascii_digit() { '0' } else { c })
            .collect::<String>()
    });

    container().child(move || {
        let row = container().layout(Flex::row().cross_alignment(CrossAlignment::Start));
        let children: Vec<Container> = pattern
            .get()
            .chars()
            .enumerate()
            .map(|(index, c)| {
                if c == '0' {
                    digit(value, index, look, color)
                } else {
                    container().child(glyph(c.to_string(), look, color))
                }
            })
            .collect();
        row.children(children)
    })
}

fn glyph(text_value: String, look: Look, color: impl Fn() -> Color + 'static) -> Text {
    text(text_value)
        .font_size(look.size)
        .line_height(LineHeight::Absolute(look.line))
        .font_weight(look.weight)
        .font_features(tabular())
        .letter_spacing(look.spacing)
        .color(color)
        .nowrap()
}

fn digit(
    value: Memo<String>,
    index: usize,
    look: Look,
    color: impl Fn() -> Color + Copy + 'static,
) -> Container {
    let shown = create_memo(move || {
        value
            .get()
            .chars()
            .nth(index)
            .and_then(|c| c.to_digit(10))
            .unwrap_or(0) as f32
    });
    let column = container()
        .layout(Flex::column())
        .translate(
            (move || Translate::new(0.0, -shown.get() * look.line))
                .transition(Transition::spring(spring())),
        )
        .children((0..10).map(move |n| container().child(glyph(n.to_string(), look, color))));
    container()
        .height(look.line)
        .overflow(Overflow::Hidden)
        .child(column)
}
