//! A picture of every output as it is just before the lock, through
//! `wlr-screencopy` on a Wayland connection of its own, since guido does not
//! speak the protocol. The lock starts on it sharp and blurs it, so the
//! compositor's cut from the session to the lock shows nothing at all; the
//! unlock sharpens it again before handing back.
//!
//! The pictures hold whatever was on screen. They live in this process's
//! memory only, are never written anywhere, and are shown sharp only before
//! the lock has been taken and after it has been undone.

use std::collections::HashMap;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::time::Duration;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

/// One output, as straight RGBA8 rows.
#[derive(Clone)]
pub struct Snapshot {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

/// The same capture, not the same pixels: comparing megabytes would be the
/// whole cost of a signal write.
impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.pixels, &other.pixels)
    }
}

/// A capture made ready: connected, with the outputs found, waiting for
/// [`capture`](Self::capture) to take the picture.
pub struct Pending {
    go: std::sync::mpsc::Sender<()>,
    done: tokio::sync::oneshot::Receiver<anyhow::Result<HashMap<String, Snapshot>>>,
}

/// Connects and finds the outputs now, while the lock still waits on the
/// bar, so only the capture itself is left for later.
pub fn prepare() -> Option<Pending> {
    let (go, wait) = std::sync::mpsc::channel();
    let (tx, done) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("allock-snapshot".to_owned())
        .spawn(move || {
            let _ = tx.send(capture_blocking(wait));
        })
        .inspect_err(|e| log::info!("no snapshot: {e}"))
        .ok()?;
    Some(Pending { go, done })
}

impl Pending {
    /// Every output that could be captured, by connector name, or nothing
    /// once `timeout` has passed: a lock is never held back for a picture.
    pub async fn capture(self, timeout: Duration) -> HashMap<String, Snapshot> {
        let _ = self.go.send(());
        match tokio::time::timeout(timeout, self.done).await {
            Ok(Ok(Ok(snapshots))) => snapshots,
            Ok(Ok(Err(e))) => {
                log::info!("no snapshot: {e}");
                HashMap::new()
            }
            Ok(Err(_)) => HashMap::new(),
            Err(_) => {
                log::info!("no snapshot: the compositor took longer than {timeout:?}");
                HashMap::new()
            }
        }
    }
}

/// Dropped unused, `go` ends the thread before it captures anything.
fn capture_blocking(
    go: std::sync::mpsc::Receiver<()>,
) -> anyhow::Result<HashMap<String, Snapshot>> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<Capture>(&connection)?;
    let qh = queue.handle();

    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ())?;
    let manager: ZwlrScreencopyManagerV1 = globals.bind(&qh, 1..=3, ())?;
    let outputs: Vec<wl_output::WlOutput> = globals.contents().with_list(|list| {
        list.iter()
            .filter(|global| global.interface == wl_output::WlOutput::interface().name)
            .map(|global| {
                globals
                    .registry()
                    .bind(global.name, global.version.min(4), &qh, ())
            })
            .collect()
    });

    let mut capture = Capture {
        names: HashMap::new(),
        frames: Vec::new(),
    };
    // The outputs' names, which wl_output v4 sends once bound.
    queue.roundtrip(&mut capture)?;
    go.recv()
        .map_err(|_| anyhow::anyhow!("the capture was not needed"))?;

    for (index, output) in outputs.iter().enumerate() {
        let frame = manager.capture_output(0, output, &qh, index);
        capture.frames.push(Frame {
            output: output.id(),
            frame,
            shm: shm.clone(),
            format: None,
            buffer: None,
            y_invert: false,
            done: None,
        });
    }

    while capture.frames.iter().any(|frame| frame.done.is_none()) {
        queue.blocking_dispatch(&mut capture)?;
    }

    let mut snapshots = HashMap::new();
    for frame in capture.frames {
        frame.frame.destroy();
        let Some(name) = capture.names.get(&frame.output).cloned() else {
            continue;
        };
        match frame.done {
            Some(Ok(snapshot)) => {
                snapshots.insert(name, snapshot);
            }
            Some(Err(e)) => log::info!("no snapshot of {name}: {e}"),
            None => {}
        }
    }
    log::debug!("snapshots of {:?}", snapshots.keys().collect::<Vec<_>>());
    Ok(snapshots)
}

struct Capture {
    names: HashMap<wayland_client::backend::ObjectId, String>,
    frames: Vec<Frame>,
}

#[derive(Clone, Copy)]
struct Format {
    format: wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
}

struct Frame {
    output: wayland_client::backend::ObjectId,
    frame: ZwlrScreencopyFrameV1,
    shm: wl_shm::WlShm,
    format: Option<Format>,
    buffer: Option<(OwnedFd, wl_buffer::WlBuffer)>,
    y_invert: bool,
    done: Option<anyhow::Result<Snapshot>>,
}

impl Frame {
    /// Hand the compositor a buffer of the size it asked for.
    fn copy(&mut self, qh: &QueueHandle<Capture>) -> anyhow::Result<()> {
        let format = self
            .format
            .ok_or_else(|| anyhow::anyhow!("no shm format offered"))?;
        let size = format.stride as usize * format.height as usize;
        let fd = unsafe { libc::memfd_create(c"allock-snapshot".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        std::fs::File::from(fd.try_clone()?).set_len(size as u64)?;
        let pool = self.shm.create_pool(fd.as_fd(), size as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            format.width as i32,
            format.height as i32,
            format.stride as i32,
            format.format,
            qh,
            (),
        );
        pool.destroy();
        self.frame.copy(&buffer);
        self.buffer = Some((fd, buffer));
        Ok(())
    }

    fn read(&mut self) -> anyhow::Result<Snapshot> {
        let format = self
            .format
            .ok_or_else(|| anyhow::anyhow!("no shm format offered"))?;
        let (fd, buffer) = self
            .buffer
            .take()
            .ok_or_else(|| anyhow::anyhow!("no buffer"))?;
        let mut raw = vec![0u8; format.stride as usize * format.height as usize];
        std::fs::File::from(fd).read_exact_at(&mut raw, 0)?;
        buffer.destroy();
        to_rgba(&raw, format, self.y_invert)
    }
}

/// The 32-bit shm formats, little-endian as wl_shm defines them, to straight
/// opaque RGBA rows.
fn to_rgba(raw: &[u8], format: Format, y_invert: bool) -> anyhow::Result<Snapshot> {
    let swap = match format.format {
        wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => true,
        wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888 => false,
        other => anyhow::bail!("unsupported shm format {other:?}"),
    };
    let (width, height) = (format.width as usize, format.height as usize);
    let mut pixels = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        let source = if y_invert { height - 1 - row } else { row };
        let start = source * format.stride as usize;
        for px in raw[start..start + width * 4].as_chunks::<4>().0 {
            let (r, b) = if swap { (px[2], px[0]) } else { (px[0], px[2]) };
            pixels.extend_from_slice(&[r, px[1], b, 255]);
        }
    }
    Ok(Snapshot {
        width: format.width,
        height: format.height,
        pixels: pixels.into(),
    })
}

impl Dispatch<ZwlrScreencopyFrameV1, usize> for Capture {
    fn event(
        capture: &mut Self,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        index: &usize,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let frame = &mut capture.frames[*index];
        use zwlr_screencopy_frame_v1::Event;
        match event {
            Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } if frame.format.is_none() => {
                frame.format = Some(Format {
                    format,
                    width,
                    height,
                    stride,
                });
                // Before v3 there is no buffer_done: the shm offer is the only one.
                if frame.frame.version() < 3
                    && let Err(e) = frame.copy(qh)
                {
                    frame.done = Some(Err(e));
                }
            }
            Event::BufferDone => {
                if let Err(e) = frame.copy(qh) {
                    frame.done = Some(Err(e));
                }
            }
            Event::Flags { flags } => {
                frame.y_invert = matches!(
                    flags,
                    WEnum::Value(flags) if flags.contains(zwlr_screencopy_frame_v1::Flags::YInvert)
                );
            }
            Event::Ready { .. } => frame.done = Some(frame.read()),
            Event::Failed => frame.done = Some(Err(anyhow::anyhow!("the compositor refused"))),
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Capture {
    fn event(
        capture: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            capture.names.insert(output.id(), name);
        }
    }
}

wayland_client::delegate_noop!(Capture: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Capture: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Capture: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(Capture: ignore ZwlrScreencopyManagerV1);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Capture {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asks the running compositor, so it is not run by default. Prints only
    /// what was captured, never the pixels.
    #[tokio::test]
    #[ignore = "captures the running session"]
    async fn captures_every_output() {
        let start = std::time::Instant::now();
        let snapshots = prepare().unwrap().capture(Duration::from_secs(2)).await;
        for (name, snapshot) in &snapshots {
            println!("{name}: {}x{}", snapshot.width, snapshot.height);
            assert_eq!(
                snapshot.pixels.len(),
                (snapshot.width * snapshot.height * 4) as usize
            );
        }
        println!("in {:?}", start.elapsed());
        assert!(!snapshots.is_empty());
    }

    #[test]
    fn xrgb_rows_become_opaque_rgba_and_y_inverted_ones_flip() {
        let format = Format {
            format: wl_shm::Format::Xrgb8888,
            width: 1,
            height: 2,
            stride: 8,
        };
        // B G R X per pixel, each row padded to its stride.
        let raw = [1, 2, 3, 0, 9, 9, 9, 9, 4, 5, 6, 0, 9, 9, 9, 9];
        let snapshot = to_rgba(&raw, format, false).unwrap();
        assert_eq!(&*snapshot.pixels, &[3, 2, 1, 255, 6, 5, 4, 255]);
        let flipped = to_rgba(&raw, format, true).unwrap();
        assert_eq!(&*flipped.pixels, &[6, 5, 4, 255, 3, 2, 1, 255]);
    }
}
