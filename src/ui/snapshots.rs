//! The screens drawn offscreen into PNGs under `target/snapshots`, to look
//! at a change without locking anything:
//!
//! ```sh
//! cargo test snapshots -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use guido::prelude::*;
use guido::testing::Headless;

use super::flow::{Flow, Phase};
use crate::Mode;
use crate::config::{Config, SessionDef};
use crate::services::auth::{self, Backend, Notice, NoticeLevel};
use crate::system::user::UserInfo;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/snapshots");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A dusk over hills, the mockup's own wallpaper, drawn by hand.
fn wallpaper() -> PathBuf {
    let path = out_dir().join("wallpaper.png");
    let (w, h) = (1600u32, 900u32);
    let sky = |t: f32| -> [f32; 3] {
        let stops = [
            (0.0, [27.0, 24.0, 52.0]),
            (0.55, [74.0, 54.0, 87.0]),
            (0.78, [196.0, 111.0, 106.0]),
            (1.0, [242.0, 164.0, 119.0]),
        ];
        let i = stops
            .iter()
            .rposition(|(at, _)| *at <= t)
            .unwrap_or(0)
            .min(2);
        let ((a, ca), (b, cb)) = (stops[i], stops[i + 1]);
        let k = ((t - a) / (b - a)).clamp(0.0, 1.0);
        [0, 1, 2].map(|c| ca[c] + (cb[c] - ca[c]) * k)
    };
    let image = ::image::RgbImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        let hill = 0.62 + 0.06 * (fx * 9.0).sin() + 0.03 * (fx * 23.0).cos();
        let sun = ((fx - 0.7).powi(2) * 3.0 + (fy - 0.6).powi(2)).sqrt() < 0.07;
        let rgb = if fy > 0.76 {
            [30.0, 22.0, 46.0]
        } else if fy > hill {
            [52.0, 38.0, 70.0]
        } else if sun {
            [255.0, 211.0, 161.0]
        } else {
            sky(fy / 0.76)
        };
        ::image::Rgb(rgb.map(|c| c as u8))
    });
    image.save(&path).unwrap();
    path
}

struct Scene {
    app: Headless,
    surface: SurfaceId,
    screen: super::Screen,
    clock: Instant,
}

impl Scene {
    fn new(mode: Mode) -> Option<Self> {
        Self::with(mode, Default::default(), |_| {})
    }

    /// With the session's picture of each output, as the lock takes it.
    /// `tune` changes the config before the screen reads it.
    fn with(
        mode: Mode,
        snapshots: super::Snapshots,
        tune: impl FnOnce(&mut Config),
    ) -> Option<Self> {
        let mut app = Headless::new()?;
        let output = app.connect_output("eDP-1");

        let mut config = Config::default();
        config.background.path = Some(wallpaper().to_string_lossy().into_owned());
        // UPower is not asked from a test; the ring is given a reading.
        config.indicators.battery = false;
        tune(&mut config);
        let config = Rc::new(config);

        let auth = auth::start(Backend::Silent, None);
        let flow = Flow::start(auth, super::flow::FOLD_MS, || {});
        let mut screen = crate::screen(config, mode, auth, flow);
        screen.snapshots = snapshots;
        screen.battery = create_signal(Some(crate::services::battery::Battery {
            percent: 59,
            plugged: false,
        }))
        .read_only();
        screen.user.set(Some(UserInfo {
            name: "zibo".to_owned(),
            display_name: "Zibo".to_owned(),
            avatar: None,
        }));
        if mode == Mode::Greeter {
            screen.users = Rc::new(vec![
                UserInfo {
                    name: "zibo".to_owned(),
                    display_name: "Zibo".to_owned(),
                    avatar: None,
                },
                UserInfo {
                    name: "guest".to_owned(),
                    display_name: "Guest".to_owned(),
                    avatar: None,
                },
            ]);
            screen.sessions = Rc::new(
                [
                    "Niri",
                    "CustomHyprland",
                    "Hyprland",
                    "Hyprland (uwsm-managed)",
                    "GNOME",
                ]
                .map(|name| SessionDef {
                    name: name.to_owned(),
                    command: vec![name.to_lowercase()],
                    env: Vec::new(),
                })
                .into(),
            );
        }

        let info = outputs()
            .get_untracked()
            .into_iter()
            .find(|info| info.id == output)?;
        let drawn = screen.clone();
        let surface = app.surface(
            SurfaceConfig::new()
                .width(0)
                .height(0)
                .anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT),
            move || super::view(drawn, info),
        );
        app.configure(surface, WIDTH, HEIGHT, 1.0);
        app.enter_output(surface, output);
        let mut scene = Self {
            app,
            surface,
            screen,
            clock: Instant::now(),
        };
        scene.run(Duration::from_millis(50));
        Some(scene)
    }

    /// Frames at 60 Hz for `span` of the scene's own time.
    fn run(&mut self, span: Duration) {
        let frames = span.as_millis() / 16;
        for _ in 0..frames.max(1) {
            self.clock += Duration::from_millis(16);
            self.app.step_at(self.clock);
            self.app.wait_for_image_decodes();
        }
    }

    /// Frames at 60 Hz in step with the wall clock, for what runs on timers.
    fn run_real(&mut self, span: Duration) {
        for _ in 0..span.as_millis() / 16 {
            std::thread::sleep(Duration::from_millis(16));
            self.clock += Duration::from_millis(16);
            self.app.step_at(self.clock);
        }
    }

    fn settle(&mut self, phase: Phase) {
        self.screen.flow.phase.set(phase);
        self.run(Duration::from_millis(1600));
    }

    fn save(&self, name: &str) {
        let pixels = self.app.read_frame(self.surface);
        let path = out_dir().join(format!("{name}.png"));
        ::image::RgbaImage::from_raw(WIDTH, HEIGHT, pixels)
            .unwrap()
            .save(&path)
            .unwrap();
        println!("{}", path.display());
    }
}

#[test]
#[ignore = "draws PNGs to look at, asserts nothing"]
fn snapshots() {
    let Some(mut lock) = Scene::new(Mode::Lock) else {
        eprintln!("no GPU adapter; nothing drawn");
        return;
    };
    // Nothing but the background: the island is up past the edge.
    lock.save("lock-0-seed");
    lock.screen.flow.grow();
    // Dropping in, on the flow's wall-clock timer.
    lock.run_real(Duration::from_millis(330));
    lock.save("lock-0b-dropping");
    lock.settle(Phase::Rest);
    lock.save("lock-1-rest");
    lock.settle(Phase::Awake);
    lock.save("lock-2-card");

    let state = lock.screen.auth.state;
    state.password.set(Secret::from("hunter2".to_owned()));
    lock.run(Duration::from_millis(600));
    lock.save("lock-3-typed");
    state.password.clear();
    state.notice.set(Some(Notice {
        level: NoticeLevel::Error,
        text: "Wrong password".to_owned(),
    }));
    state.tally.set(Some(crate::system::faillock::Tally {
        failed: 2,
        deny: 3,
        lockout: Some(Duration::from_secs(600)),
    }));
    lock.screen.flow.caps_at_refusal.set(true);
    lock.run(Duration::from_millis(800));
    lock.save("lock-4-refused");
    state.password.set(Secret::from("h".to_owned()));
    lock.run(Duration::from_millis(400));
    lock.save("lock-4b-last-try");
    state.password.clear();
    lock.screen.flow.caps_at_refusal.set(false);
    lock.screen.flow.zone.set(Some(super::flow::Zone::Power));
    lock.run(Duration::from_millis(800));
    lock.save("lock-5-power");
    lock.screen.flow.zone.set(None);
    state.locked.set(Some(crate::system::faillock::Lock::Until(
        std::time::SystemTime::now() + Duration::from_secs(272),
    )));
    lock.run(Duration::from_millis(800));
    lock.save("lock-5b-lockout");
    lock.settle(Phase::Rest);
    lock.save("lock-5c-lockout-rest");
    state.locked.set(None);
    lock.settle(Phase::Awake);
    // The flow's own timers run on the wall clock: the tick shows for 600 ms.
    // The tick is drawn on the wall clock: 150 ms of wait, then 500 of pen.
    state.authenticated.set(true);
    lock.run_real(Duration::from_millis(250));
    lock.save("lock-6-ok-drawing");
    lock.run_real(Duration::from_millis(330));
    lock.save("lock-6-ok");
    // Going up into the edge.
    lock.run_real(Duration::from_millis(200));
    lock.save("lock-7-going");

    // One application at a time: two on one thread share guido's globals.
    drop(lock);

    // The session's own picture stands in for the snapshot: sharp on the
    // first frame, blurred once the island drops in.
    let desk = ::image::open(wallpaper())
        .unwrap()
        .resize_exact(WIDTH, HEIGHT, ::image::imageops::FilterType::Triangle)
        .into_rgba8();
    let snapshot = crate::services::snapshot::Snapshot {
        width: WIDTH,
        height: HEIGHT,
        pixels: desk.into_raw().into(),
    };
    let snapshots: super::Snapshots = std::sync::Arc::new([("eDP-1".to_owned(), snapshot)].into());
    let mut over = Scene::with(Mode::Lock, snapshots.clone(), |_| {}).unwrap();
    over.save("snapshot-0-seed");
    over.screen.flow.grow();
    // Dropping in and opening into the card, at its own size, uncovered.
    over.run_real(Duration::from_millis(480));
    over.save("snapshot-0b-growing");
    over.run(Duration::from_millis(1800));
    over.save("snapshot-1-card");

    drop(over);

    // A translucent island softens the session through it.
    let mut glass = Scene::with(Mode::Lock, snapshots, |config| {
        config.appearance.material = ashell_ds::ui::Material::Glass;
    })
    .unwrap();
    glass.save("translucent-0-seed");
    glass.screen.flow.grow();
    glass.settle(Phase::Rest);
    glass.save("translucent-1-rest");
    glass.settle(Phase::Awake);
    glass.run(Duration::from_millis(1800));
    glass.save("translucent-2-card");
    drop(glass);

    // Another theme, flavour and accent, light, compact, round and glass:
    // everything follows `[appearance]`.
    let mut themed = Scene::with(Mode::Lock, Default::default(), |config| {
        config.appearance = toml::from_str(
            "theme = \"Catppuccin\"\nmode = \"light\"\naccent = \"Mauve\"\n\
             density = \"compact\"\nshape = \"round\"\nmaterial = \"glass\"",
        )
        .unwrap();
    })
    .unwrap();
    themed.settle(Phase::Rest);
    themed.save("themed-1-rest");
    themed.settle(Phase::Awake);
    themed.save("themed-2-card");
    drop(themed);

    let mut greeter = Scene::new(Mode::Greeter).unwrap();
    greeter.settle(Phase::Rest);
    greeter.save("greeter-1-rest");
    greeter.settle(Phase::Awake);
    greeter.save("greeter-2-card");
    greeter
        .screen
        .flow
        .zone
        .set(Some(super::flow::Zone::Session));
    greeter.run(Duration::from_millis(800));
    greeter.save("greeter-2b-sessions");
    greeter.screen.flow.zone.set(None);
    greeter.run(Duration::from_millis(800));
    greeter.settle(Phase::Gone);
    greeter.save("greeter-3-gone");
}

/// The lock in action, as raw RGBA frames at 60 fps in
/// `target/snapshots/demo.rgba`, for `scripts/demo.sh` to make the README's
/// animation of. Paced on the wall clock, as the flow's timers are.
#[test]
#[ignore = "records a video, asserts nothing"]
fn demo() {
    use std::io::Write;

    const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 60);

    struct Recorder {
        out: std::io::BufWriter<std::fs::File>,
        next: Instant,
    }

    impl Recorder {
        fn record(&mut self, scene: &mut Scene, span: Duration) {
            let end = self.next + span;
            while self.next < end {
                if let Some(wait) = self.next.checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                }
                scene.clock = scene.clock.max(self.next);
                scene.app.step_at(scene.clock);
                self.out
                    .write_all(&scene.app.read_frame(scene.surface))
                    .unwrap();
                self.next += FRAME;
            }
        }
    }

    // The session's own picture, as the lock takes it: sharp, then blurred.
    let desk = ::image::open(wallpaper())
        .unwrap()
        .resize_exact(WIDTH, HEIGHT, ::image::imageops::FilterType::Triangle)
        .into_rgba8();
    let snapshot = crate::services::snapshot::Snapshot {
        width: WIDTH,
        height: HEIGHT,
        pixels: desk.into_raw().into(),
    };
    let snapshots: super::Snapshots = std::sync::Arc::new([("eDP-1".to_owned(), snapshot)].into());
    let Some(mut lock) = Scene::with(Mode::Lock, snapshots, |config| {
        config.appearance.material = ashell_ds::ui::Material::Translucent;
    }) else {
        eprintln!("no GPU adapter; nothing recorded");
        return;
    };
    let path = out_dir().join("demo.rgba");
    let mut rec = Recorder {
        out: std::io::BufWriter::new(std::fs::File::create(&path).unwrap()),
        next: Instant::now(),
    };
    let state = lock.screen.auth.state;
    let flow = lock.screen.flow;
    let typing = |rec: &mut Recorder, lock: &mut Scene, word: &str| {
        for n in 1..=word.chars().count() {
            state
                .password
                .set(Secret::from(word.chars().take(n).collect::<String>()));
            rec.record(lock, Duration::from_millis(130));
        }
    };

    // The session, then the island dropping in, at rest.
    rec.record(&mut lock, Duration::from_millis(600));
    flow.phase.set(Phase::Rest);
    rec.record(&mut lock, Duration::from_millis(1600));
    // A key wakes it into the card.
    flow.wake();
    rec.record(&mut lock, Duration::from_millis(1300));
    // A wrong password: the island shakes and glows red.
    typing(&mut rec, &mut lock, "hunter2");
    rec.record(&mut lock, Duration::from_millis(300));
    state.busy.set(true);
    rec.record(&mut lock, Duration::from_millis(500));
    state.busy.set(false);
    state.password.clear();
    state.notice.set(Some(Notice {
        level: NoticeLevel::Error,
        text: "Wrong password".to_owned(),
    }));
    state.tally.set(Some(crate::system::faillock::Tally {
        failed: 1,
        deny: 3,
        lockout: Some(Duration::from_secs(600)),
    }));
    state.rejections.update(|n| *n += 1);
    rec.record(&mut lock, Duration::from_millis(1500));
    // The right one: the tick, and the island goes back up.
    typing(&mut rec, &mut lock, "correct horse");
    rec.record(&mut lock, Duration::from_millis(300));
    state.busy.set(true);
    rec.record(&mut lock, Duration::from_millis(500));
    state.busy.set(false);
    state.authenticated.set(true);
    rec.record(&mut lock, Duration::from_millis(2600));
    rec.out.flush().unwrap();
    println!("{}", path.display());
}
