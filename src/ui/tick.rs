//! The tick an accepted answer gets, drawn as if by a pen: Lucide's check,
//! revealed along its length by a dash whose offset runs down to nothing.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use guido::prelude::*;
use guido::widget_prelude::*;

use ashell_ds::ui::Motion;

/// The check's corners in Lucide's 24-unit box, in the order it is drawn.
const POINTS: [(f32, f32); 3] = [(20.0, 6.0), (9.0, 17.0), (4.0, 12.0)];
const DELAY: Duration = Duration::from_millis(150);
const DRAW: Duration = Duration::from_millis(500);
const FRAME: Duration = Duration::from_millis(16);

struct Tick {
    size: f32,
    stroke: f32,
    color: Color,
    /// How much of the stroke is drawn, eased, from 0 to 1.
    drawn: Signal<f32>,
}

pub fn tick(size: f32, stroke: f32, color: Color) -> impl Widget {
    // Without full motion it is there at once, already drawn.
    if ashell_ds::ui().motion != Motion::Full {
        return Tick {
            size,
            stroke,
            color,
            drawn: create_signal(1.0f32).into(),
        };
    }
    let drawn = create_signal(0.0f32);
    let start = Instant::now() + DELAY;
    let handle: Rc<Cell<Option<TimerHandle>>> = Rc::default();
    let own = handle.clone();
    handle.set(Some(set_interval(FRAME, move || {
        let t = Instant::now()
            .saturating_duration_since(start)
            .as_secs_f32()
            / DRAW.as_secs_f32();
        let t = t.clamp(0.0, 1.0);
        // A cubic ease-out: quick out of the start, slow into the end.
        drawn.set(1.0 - (1.0 - t).powi(3));
        if t >= 1.0
            && let Some(handle) = own.take()
        {
            handle.cancel();
        }
    })));
    Tick {
        size,
        stroke,
        color,
        drawn: drawn.into(),
    }
}

impl Widget for Tick {
    fn layout(&mut self, _ctx: &mut LayoutCtx, constraints: Constraints) -> Size {
        constraints.constrain(Size::new(self.size, self.size))
    }

    fn paint(&self, ctx: &mut PaintContext) {
        let drawn = self.drawn.get();
        if drawn <= 0.0 {
            return;
        }
        let scale = self.size / 24.0;
        let at = |(x, y): (f32, f32)| (x * scale, y * scale);
        let path = Path::new()
            .move_to(at(POINTS[0]))
            .line_to(at(POINTS[1]))
            .line_to(at(POINTS[2]));
        let length = POINTS
            .windows(2)
            .map(|pair| {
                let (a, b) = (pair[0], pair[1]);
                ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
            })
            .sum::<f32>()
            * scale;
        let stroke = Stroke::new(self.stroke, self.color)
            .caps(Cap::Round)
            .joins(Join::Round);
        // A dash as long as the line, and as long a gap: the offset slides the
        // gap off the line as it is drawn.
        let stroke = if drawn < 1.0 {
            stroke.dashes(&[length, length], length * (1.0 - drawn))
        } else {
            stroke
        };
        ctx.draw_path(&path, None, stroke);
    }
}
