//! Colours resolved once from `[appearance]` and shared through guido's
//! context.

use guido::prelude::Color;

use crate::config::Appearance;

#[derive(Clone, Copy, Debug)]
pub struct ThemeColors {
    pub text: Color,
    /// Dimmed text for secondary labels (date, hints).
    pub text_weak: Color,
    pub background: Color,
    /// Slightly lifted background for the input field inside the card.
    pub background_weak: Color,
    pub primary: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    /// Opacity of the login card.
    pub opacity: f32,
}

pub fn init(appearance: &Appearance) -> ThemeColors {
    let text = appearance.text_color.base();
    let background = appearance.background_color.base();
    ThemeColors {
        text,
        text_weak: with_alpha(text, 0.65),
        background,
        background_weak: appearance
            .background_color
            .weak()
            .unwrap_or_else(|| lighten(background, 0.08)),
        primary: appearance.primary_color.base(),
        success: appearance.success_color.base(),
        warning: appearance.warning_color.base(),
        danger: appearance.danger_color.base(),
        opacity: appearance.opacity.clamp(0.0, 1.0),
    }
}

pub fn with_alpha(color: Color, alpha: f32) -> Color {
    Color::rgba(color.r, color.g, color.b, alpha)
}

fn lighten(color: Color, amount: f32) -> Color {
    Color::rgba(
        (color.r + amount).min(1.0),
        (color.g + amount).min(1.0),
        (color.b + amount).min(1.0),
        color.a,
    )
}
