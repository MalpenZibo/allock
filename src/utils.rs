//! Shared helpers. `IndicatorState` lives here because the ported ashell
//! services import it as `crate::utils::IndicatorState` — keeping the path
//! means those files stay byte-identical to upstream.

use guido::prelude::Color;

use crate::theme::ThemeColors;

/// Mirrors ashell's own enum. All four variants exist because the ported
/// services are typed against it, even though the battery — allock's only
/// consumer — never reports `Warning`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum IndicatorState {
    Normal,
    Success,
    Warning,
    Danger,
}

impl IndicatorState {
    pub fn color(self, theme: &ThemeColors) -> Color {
        match self {
            IndicatorState::Normal => theme.text,
            IndicatorState::Success => theme.success,
            IndicatorState::Warning => theme.warning,
            IndicatorState::Danger => theme.danger,
        }
    }
}
