//! Line icons, Lucide's, as SVGs drawn in white and tinted where they are used.

use std::cell::RefCell;
use std::collections::HashMap;

use guido::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Lock,
    Arrow,
    Eye,
    EyeOff,
    Finger,
    Caps,
    Check,
    Power,
    Refresh,
    Moon,
    Monitor,
    User,
    ChevronDown,
}

impl Icon {
    fn body(self) -> &'static str {
        match self {
            Icon::Lock => {
                r#"<rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>"#
            }
            Icon::Arrow => r#"<path d="M5 12h14M13 6l6 6-6 6"/>"#,
            Icon::Eye => {
                r#"<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/>"#
            }
            Icon::EyeOff => {
                r#"<path d="M9.9 4.24A9 9 0 0 1 12 4c6.5 0 10 8 10 8a18.5 18.5 0 0 1-2.16 3.19M6.6 6.6A18.4 18.4 0 0 0 2 12s3.5 8 10 8a9.1 9.1 0 0 0 5.4-1.6"/><path d="m2 2 20 20"/>"#
            }
            Icon::Finger => {
                r#"<path d="M12 10a2 2 0 0 0-2 2c0 1.02-.1 2.51-.26 4"/><path d="M14 13.12c0 2.38 0 6.38-1 8.88"/><path d="M17.29 21.02c.12-.6.43-2.3.5-3.02"/><path d="M2 12a10 10 0 0 1 18-6"/><path d="M2 16h.01"/><path d="M21.8 16c.2-2 .131-5.354 0-6"/><path d="M5 19.5C5.5 18 6 15 6 12a6 6 0 0 1 .34-2"/><path d="M8.65 22c.21-.66.45-1.32.57-2"/><path d="M9 6.8a6 6 0 0 1 9 5.2v2"/>"#
            }
            Icon::Caps => r#"<path d="m12 4 7 8h-4v5H9v-5H5Z"/><path d="M9 21h6"/>"#,
            Icon::Check => r#"<polyline points="20 6 9 17 4 12"/>"#,
            Icon::Power => {
                r#"<path d="M18.36 6.64a9 9 0 1 1-12.73 0"/><line x1="12" y1="2" x2="12" y2="12"/>"#
            }
            Icon::Refresh => r#"<path d="M21 12a9 9 0 1 1-3-6.7L21 8"/><path d="M21 3v5h-5"/>"#,
            Icon::Moon => r#"<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>"#,
            Icon::Monitor => {
                r#"<rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/>"#
            }
            Icon::User => r#"<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>"#,
            Icon::ChevronDown => r#"<polyline points="6 9 12 15 18 9"/>"#,
        }
    }

    /// One source per icon and weight, so every use shares one texture.
    fn source(self, weight: f32) -> ImageSource {
        thread_local! {
            static SOURCES: RefCell<HashMap<(Icon, u32), ImageSource>> = RefCell::default();
        }
        SOURCES.with_borrow_mut(|sources| {
            sources
                .entry((self, weight.to_bits()))
                .or_insert_with(|| {
                    let svg = format!(
                        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="{weight}" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
                        self.body()
                    );
                    ImageSource::SvgBytes(svg.into_bytes().into())
                })
                .clone()
        })
    }
}

pub fn icon<M>(kind: Icon, size: f32, color: impl IntoSignal<Color, M>) -> Container {
    weighted(kind, size, 2.0, color)
}

/// With a stroke other than Lucide's 2 units in 24.
pub fn weighted<M>(
    kind: Icon,
    size: f32,
    weight: f32,
    color: impl IntoSignal<Color, M>,
) -> Container {
    container()
        .width(size)
        .height(size)
        .child(image(kind.source(weight)).tint(color))
}
