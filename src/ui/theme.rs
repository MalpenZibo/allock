//! Colours resolved once from `[appearance]`, plus the mockup's fixed tints.

use guido::prelude::Color;

use crate::config::{Appearance, color};

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub island: Color,
    pub background: Color,
    pub text: Color,
    /// Secondary text and icons.
    pub sub: Color,
    /// Hints and labels.
    pub dim: Color,
    /// A control's resting fill on the island.
    pub surface: Color,
    /// Its hovered or chosen fill.
    pub surface2: Color,
    pub accent: Color,
    /// Text on the accent.
    pub accent_ink: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
}

impl Theme {
    pub fn new(appearance: &Appearance) -> Self {
        let text = color(appearance.text_color);
        Self {
            island: color(appearance.island_color),
            background: color(appearance.background_color),
            text,
            sub: text.with_alpha(0.78),
            dim: text.with_alpha(0.48),
            surface: Color::WHITE.with_alpha(0.065),
            surface2: Color::WHITE.with_alpha(0.11),
            accent: color(appearance.primary_color),
            accent_ink: Color::rgb(0.118, 0.118, 0.18),
            success: color(appearance.success_color),
            warning: color(appearance.warning_color),
            danger: color(appearance.danger_color),
        }
    }
}
