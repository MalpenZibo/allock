//! Shared helpers. `IndicatorState` lives here because the services import it
//! as `crate::utils::IndicatorState`.

use guido::prelude::Color;

use crate::theme::ThemeColors;

/// All four variants exist because the services are typed against them, even
/// though the battery — allock's only consumer — never reports `Warning`.
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
