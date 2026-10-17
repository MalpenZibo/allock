//! ashell's theme, with the colours `[appearance]` gives. The rest (sizes,
//! radii, the controls' translucent fills) is ashell's own, so the two look
//! like one shell.

use ashell_ui::Theme;

use crate::config::{Appearance, color};

pub fn from_config(appearance: &Appearance) -> Theme {
    let mut theme = Theme::default();
    let colors = &mut theme.colors;
    let text = color(appearance.text_color);
    colors.island = color(appearance.island_color);
    colors.text = text;
    colors.sub = text.with_alpha(0.78);
    colors.dim = text.with_alpha(0.48);
    colors.accent = color(appearance.primary_color);
    colors.success = color(appearance.success_color);
    colors.warning = color(appearance.warning_color);
    colors.danger = color(appearance.danger_color);
    theme
}
