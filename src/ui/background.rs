//! What the island sits on: the wallpaper, or a snapshot of the session,
//! blurred and darkened. Resting darkens it further. It is sharp when it has
//! to match what is on the other side of a cut: a snapshot as the lock
//! starts and as it hands back, and the greeter's wallpaper as the session,
//! which starts on the same picture, takes over.

use guido::prelude::*;

use super::Screen;
use super::flow::Phase;
use crate::Mode;
use crate::config::BackgroundFit;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Look {
    blur: f32,
    dim: f32,
    scale: f32,
}

const SHARP: Look = Look {
    blur: 0.0,
    dim: 0.0,
    scale: 1.0,
};

const EASE_MS: f32 = 1100.0;
const FADE_IN_MS: f32 = 400.0;
/// Resting adds this much blur, and darkens to this.
const REST_BLUR: f32 = 8.0;
const REST_DIM: f32 = 0.68;
/// Blurred edges pull in the colour beyond the picture; a little zoom hides them.
const BLUR_ZOOM: f32 = 1.06;

pub fn view(screen: &Screen, output: &OutputInfo) -> Option<Container> {
    let snapshot = output
        .name
        .as_ref()
        .and_then(|name| screen.snapshots.get(name))
        .map(|snapshot| ImageSource::Rgba {
            width: snapshot.width,
            height: snapshot.height,
            pixels: snapshot.pixels.clone(),
        });
    let is_snapshot = snapshot.is_some();
    let (source, fit) = match snapshot {
        // The output's own pixels: they fill it exactly.
        Some(source) => (source, ContentFit::Fill),
        None => (
            ImageSource::Path(screen.config.background.resolved_path()?),
            match screen.config.background.fit {
                BackgroundFit::Cover => ContentFit::Cover,
                BackgroundFit::Contain => ContentFit::Contain,
                BackgroundFit::Stretch => ContentFit::Fill,
            },
        ),
    };

    let (blur, dim) = (
        screen.config.background.blur.max(0.0),
        screen.config.background.dim.clamp(0.0, 1.0),
    );
    let phase = screen.flow.phase;
    let greeter = screen.mode == Mode::Greeter;

    let look = create_memo(move || {
        let zoom = if blur > 0.0 { BLUR_ZOOM } else { 1.0 };
        match phase.get() {
            Phase::Seed if is_snapshot => SHARP,
            Phase::Gone if is_snapshot || greeter => SHARP,
            Phase::Awake | Phase::Ok => Look {
                blur,
                dim,
                scale: zoom,
            },
            _ => Look {
                blur: if blur > 0.0 { blur + REST_BLUR } else { 0.0 },
                dim: dim.max(REST_DIM),
                scale: zoom,
            },
        }
    });
    let ease = || Transition::new(EASE_MS, TimingFunction::EaseInOut);

    // A file is decoded off the frame, so the first frame (the seed) goes out
    // without waiting for it; a snapshot is there from the first frame.
    let picture = image(source).content_fit(fit);
    let ready = picture.ready();

    Some(
        container()
            .width(fill())
            .height(fill())
            .layout(ZStack::new())
            .opacity((move || if ready.get() { 1.0 } else { 0.0 }).transition(FADE_IN_MS))
            .child(
                container()
                    .width(fill())
                    .height(fill())
                    .layout(Flex::column().center())
                    .blur((move || look.get().blur).transition(ease()))
                    .scale((move || Scale::uniform(look.get().scale)).transition(ease()))
                    .child(picture),
            )
            .child(
                container()
                    .width(fill())
                    .height(fill())
                    .background(Color::BLACK)
                    .opacity((move || look.get().dim).transition(ease())),
            ),
    )
}
