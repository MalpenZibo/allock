//! Shared helpers. The services import `crate::utils::IndicatorState`.

use guido::prelude::Color;

use crate::theme::ThemeColors;

/// The services are typed against all four, though the battery never reports `Warning`.
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
