//! Files dragged onto the window from a file manager, on Linux Wayland.
//!
//! winit (through eframe) reports dropped files on X11, Windows and macOS but not on Wayland, so
//! a drag from Dolphin or Files did nothing there. [`listen`] joins the window's own Wayland
//! connection (the display handle eframe gives the app) on a thread of its own: it accepts drags
//! that carry files (`text/uri-list`), reports where they hover, and on a drop reads the paths
//! and hands them over. Everything else (text, images from a browser) is declined, as it would be
//! by an application that takes only files.
//!
//! The thread waits on the connection with a timeout, so [`Listener`] stops it (when dropped)
//! before the window and its connection go away.

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::io::Read;
use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use raw_window_handle::RawDisplayHandle;
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{self, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, event_created_child};

/// What happens to a drag of files over the window. Positions are in the window's logical
/// pixels (egui points before the app's own zoom).
#[derive(Clone, Debug, PartialEq)]
pub enum DropEvent {
    /// Files are being dragged over the window, the pointer here.
    Hover { x: f32, y: f32 },
    /// The drag left the window (or was cancelled).
    Leave,
    /// Files were dropped here.
    Drop { paths: Vec<String>, x: f32, y: f32 },
}

/// The listening thread; dropping it stops the thread and waits for it.
pub struct Listener {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

const URI_LIST: &str = "text/uri-list";

/// Listen for files dropped on the window whose display connection `display` is (a Wayland one;
/// anything else, or a failure, gives None and the toolkit's own drops are all there is).
/// `display` must stay connected while the [`Listener`] lives: keep the listener in the app and
/// drop it before the window closes (eframe's `on_exit`).
pub fn listen(display: RawDisplayHandle, on_event: Box<dyn FnMut(DropEvent) + Send>) -> Option<Listener> {
    let RawDisplayHandle::Wayland(h) = display else { return None };
    // SAFETY: the pointer is the live `wl_display` of the app's window (eframe's display handle);
    // the caller keeps it connected while the listener lives, and the listener's thread stops
    // before it is dropped. A foreign display is not disconnected when this backend goes.
    let backend = unsafe { Backend::from_foreign_display(h.display.as_ptr().cast()) };
    let conn = Connection::from_backend(backend);
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::Builder::new()
        .name("file-drop".into())
        .spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(conn, on_event, &flag)));
            if let Ok(Err(e)) = r {
                log::warn!("file drops from other applications are not available: {e}");
            }
        })
        .ok()?;
    Some(Listener { stop, thread: Some(thread) })
}

struct State {
    on_event: Box<dyn FnMut(DropEvent) + Send>,
    manager: Option<WlDataDeviceManager>,
    seats: Vec<WlSeat>,
    devices: Vec<WlDataDevice>,
    /// The types each offer carries (announced right after it is made).
    mimes: HashMap<ObjectId, Vec<String>>,
    /// The offer being dragged over the window, if it carries files.
    current: Option<WlDataOffer>,
    at: (f32, f32),
    conn: Connection,
}

fn run(conn: Connection, on_event: Box<dyn FnMut(DropEvent) + Send>, stop: &AtomicBool) -> Result<(), String> {
    let mut queue: EventQueue<State> = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut st =
        State { on_event, manager: None, seats: Vec::new(), devices: Vec::new(), mimes: HashMap::new(), current: None, at: (0.0, 0.0), conn: conn.clone() };
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    let Some(manager) = st.manager.clone() else { return Err("the compositor offers no drag and drop".into()) };
    for seat in st.seats.clone() {
        st.devices.push(manager.get_data_device(&seat, &qh, ()));
    }
    while !stop.load(Ordering::Relaxed) {
        queue.dispatch_pending(&mut st).map_err(|e| e.to_string())?;
        queue.flush().map_err(|e| e.to_string())?;
        let Some(guard) = queue.prepare_read() else { continue };
        let ready = {
            let fd = guard.connection_fd();
            let mut fds = [rustix::event::PollFd::new(&fd, rustix::event::PollFlags::IN)];
            let timeout = rustix::event::Timespec { tv_sec: 0, tv_nsec: 100_000_000 };
            rustix::event::poll(&mut fds, Some(&timeout)).unwrap_or(0) > 0
        };
        if ready {
            // (another thread may have read them first: then there is nothing more to read)
            let _ = guard.read();
        }
    }
    for d in st.devices.drain(..) {
        if d.version() >= 2 {
            d.release();
        }
    }
    let _ = conn.flush();
    Ok(())
}

/// The local paths in a `text/uri-list` (`file://` URIs, percent-encoded; comments skipped).
pub fn parse_uri_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("file://"))
        // `file:///path` or `file://host/path`
        .filter_map(|rest| rest.find('/').map(|i| &rest[i..]))
        .map(percent_decode)
        .filter(|p| !p.is_empty())
        .collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (b[i], b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, ..) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl State {
    /// Read the dropped offer's paths (the source writes them into a pipe).
    fn receive(&mut self, offer: &WlDataOffer) -> Vec<String> {
        let Ok((mut reader, writer)) = std::io::pipe() else { return Vec::new() };
        offer.receive(URI_LIST.into(), writer.as_fd());
        drop(writer);
        if self.conn.flush().is_err() {
            return Vec::new();
        }
        let mut text = String::new();
        // (at most a few MB of paths)
        let _ = (&mut reader).take(16 << 20).read_to_string(&mut text);
        parse_uri_list(&text)
    }

    fn end_offer(&mut self) {
        if let Some(o) = self.current.take() {
            self.mimes.remove(&o.id());
            o.destroy();
        }
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(st: &mut Self, registry: &WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        let wl_registry::Event::Global { name, interface, version } = event else { return };
        match interface.as_str() {
            "wl_data_device_manager" if st.manager.is_none() => st.manager = Some(registry.bind(name, version.min(3), qh, ())),
            "wl_seat" => st.seats.push(registry.bind(name, version.min(5), qh, ())),
            _ => {}
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(_: &mut Self, _: &WlSeat, _: <WlSeat as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(_: &mut Self, _: &WlDataDeviceManager, _: wl_data_device_manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(st: &mut Self, offer: &WlDataOffer, event: wl_data_offer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            st.mimes.entry(offer.id()).or_default().push(mime_type);
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(st: &mut Self, _: &WlDataDevice, event: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_data_device::Event::Enter { serial, x, y, id, .. } => {
                st.end_offer();
                let Some(offer) = id else { return };
                let files = st.mimes.get(&offer.id()).is_some_and(|m| m.iter().any(|t| t == URI_LIST));
                if !files {
                    // not files: declined (and the clipboard's offers are not ours to keep)
                    offer.accept(serial, None);
                    st.mimes.remove(&offer.id());
                    return;
                }
                offer.accept(serial, Some(URI_LIST.into()));
                if offer.version() >= 3 {
                    offer.set_actions(wl_data_device_manager::DndAction::Copy, wl_data_device_manager::DndAction::Copy);
                }
                st.at = (x as f32, y as f32);
                st.current = Some(offer);
                let at = st.at;
                (st.on_event)(DropEvent::Hover { x: at.0, y: at.1 });
            }
            wl_data_device::Event::Motion { x, y, .. } if st.current.is_some() => {
                st.at = (x as f32, y as f32);
                (st.on_event)(DropEvent::Hover { x: x as f32, y: y as f32 });
            }
            wl_data_device::Event::Leave if st.current.is_some() => {
                st.end_offer();
                (st.on_event)(DropEvent::Leave);
            }
            wl_data_device::Event::Drop => {
                let Some(offer) = st.current.clone() else { return };
                let paths = st.receive(&offer);
                if offer.version() >= 3 {
                    offer.finish();
                }
                st.end_offer();
                let at = st.at;
                (st.on_event)(DropEvent::Drop { paths, x: at.0, y: at.1 });
            }
            // the clipboard's selection: not ours
            wl_data_device::Event::Selection { id: Some(offer) } => {
                st.mimes.remove(&offer.id());
                offer.destroy();
            }
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_lists_give_local_paths() {
        let list = "# from Dolphin\r\nfile:///mnt/LocalDisk/Editing/Cats%20Dancing%20Battle%20-%20(%20That%20Or%20That).mp4\r\nfile://lacegaruda/home/lace/a%26b.wav\r\nhttps://example.com/x.mp4\r\n\r\nfile:///tmp/%C3%A9t%C3%A9.mov\n";
        assert_eq!(parse_uri_list(list), ["/mnt/LocalDisk/Editing/Cats Dancing Battle - ( That Or That).mp4", "/home/lace/a&b.wav", "/tmp/été.mov"]);
        assert!(parse_uri_list("").is_empty());
        assert_eq!(parse_uri_list("file:///bad%zzname%"), ["/bad%zzname%"], "a stray % is kept");
    }
}
