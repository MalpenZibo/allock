//! Design tokens, after Bento's scales. The display sizes are larger, for the clock.

#![allow(dead_code)] // a scale is kept whole, used or not

use guido::prelude::*;

pub mod space {
    pub const S0: f32 = 0.0;
    pub const S4: f32 = 4.0;
    pub const S8: f32 = 8.0;
    pub const S12: f32 = 12.0;
    pub const S16: f32 = 16.0;
    pub const S24: f32 = 24.0;
    pub const S32: f32 = 32.0;
    pub const S40: f32 = 40.0;
    pub const S80: f32 = 80.0;
}

pub mod radius {
    pub const R4: f32 = 4.0;
    pub const R8: f32 = 8.0;
    pub const R16: f32 = 16.0;
}

pub mod icon_size {
    pub const I16: f32 = 16.0;
    pub const I24: f32 = 24.0;
    pub const I32: f32 = 32.0;
    pub const I40: f32 = 40.0;
}

#[derive(Clone, Copy, Debug)]
pub struct Type {
    pub size: f32,
    pub line: f32,
    pub weight: FontWeight,
}

// Bento's medium and semibold, at the two weights every family has: a weight
// the family lacks leaves the family (MalpenZibo/guido#575).
const REGULAR: FontWeight = FontWeight::NORMAL;
const STRONG: FontWeight = FontWeight::BOLD;

const fn ty(size: f32, line: f32, weight: FontWeight) -> Type {
    Type { size, line, weight }
}

pub const BODY_S: Type = ty(12.0, 18.0, REGULAR);
pub const BODY_M: Type = ty(14.0, 20.0, REGULAR);
pub const BODY_L: Type = ty(16.0, 24.0, REGULAR);
pub const LABEL_S: Type = ty(12.0, 18.0, REGULAR);
pub const LABEL_M: Type = ty(14.0, 20.0, REGULAR);
pub const LABEL_L: Type = ty(16.0, 24.0, REGULAR);
pub const TITLE_S: Type = ty(14.0, 16.0, STRONG);
pub const TITLE_M: Type = ty(16.0, 18.0, STRONG);
pub const TITLE_L: Type = ty(22.0, 24.0, STRONG);
pub const HEADLINE_S: Type = ty(32.0, 36.0, STRONG);
pub const HEADLINE_M: Type = ty(36.0, 40.0, STRONG);
pub const HEADLINE_L: Type = ty(40.0, 44.0, STRONG);
pub const DISPLAY_S: Type = ty(48.0, 56.0, STRONG);
pub const DISPLAY_M: Type = ty(64.0, 72.0, STRONG);
pub const DISPLAY_L: Type = ty(96.0, 104.0, STRONG);

pub trait Typography {
    fn typography(self, ty: Type) -> Self;
}

impl Typography for Text {
    fn typography(self, ty: Type) -> Self {
        self.font_size(ty.size)
            .line_height(LineHeight::Absolute(ty.line))
            .font_weight(ty.weight)
    }
}
