//! App shell: wires baa-web's browser engine (BaaCore), baa-model's store and
//! baa-ui's root view into one window, and keeps one running instance per
//! profile (PLAN.md §6).

#[cfg(not(target_os = "macos"))]
compile_error!("baa runs on macOS only until the //baa Windows port");

use std::{
    cell::RefCell,
    fs::File,
    os::unix::{ffi::OsStrExt as _, fs::PermissionsExt as _, io::AsRawFd as _, net::{UnixListener, UnixStream}},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{atomic::{AtomicBool, Ordering}, Arc},
};

use baa_model::{Settings, Store, WindowFrame};
use baa_ui::{AgentPeer, Browser, LilBaa};
use baa_web::{Download, DownloadState, Downloads, WebHost};
use futures::{channel::mpsc, StreamExt as _};
use gpui::{
    actions, point, px, size, AnyWindowHandle, App, AppContext as _, Bounds, DisplayId, Entity, KeyBinding,
    MenuItem, Pixels, TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowOptions,
};

mod menus;
mod updater;

actions!(baa_app, [Quit, DockNewWindow, DockNewIncognito, DockShowLil, DockCloseLil]);

/// BaaCore: Chromium's own main runs inside gpui's `[NSApp run]`, so the
/// order here is load-bearing (PLAN-chromium-phase2.md "Results").
fn main() {
    // `baa mcp` (Claude Code's stdio server) bridges to the running app: no engine, no gpui, no lock.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        return mcp_bridge(&data_dir());
    }
    // First: loading the framework installs PartitionAlloc's malloc zone,
    // which is only safe while the process has one thread.
    let core = baa_web::CoreRuntime::load();
    env_logger::init();
    let dir = data_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        fail("creating the data directory", e);
    }
    let args: Vec<String> = std::env::args().collect();
    let urls: Vec<String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).cloned().collect();

    // Before Chrome starts, so Chrome's own process singleton never meets a second instance.
    let Some(_lock) = single_instance(&dir, &urls) else { return };

    let chromium = dir.join("chromium");
    if let Err(e) = std::fs::create_dir_all(&chromium) {
        fail("creating the Chromium directory", e);
    }
    // Chromium puts the disk cache of a user data dir under Application Support
    // at the same path under Caches, and its sandbox setup fails (logged) when
    // that folder does not exist yet.
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if let Ok(rest) = chromium.strip_prefix(home.join("Library/Application Support")) {
            let _ = std::fs::create_dir_all(home.join("Library/Caches").join(rest));
        }
    }
    baa_web::migrate_cef_profiles(&dir);
    baa_web::purge_deleted_profiles(&chromium);
    baa_web::share_native_messaging_hosts(&chromium);
    core.install(baa_web::chrome_argv(&args, &chromium));

    let store = Store::open(dir.join("baa.db")).unwrap_or_else(|e| fail("opening baa.db", e));
    let settings = store.load_settings().unwrap_or_default();
    let (tx, rx) = mpsc::unbounded::<String>();
    for url in urls {
        tx.unbounded_send(url).ok();
    }
    listen(&dir, tx.clone());
    let (agent_tx, agent_rx) = mpsc::unbounded::<AgentPeer>();
    let mut agent = AgentSocket { path: dir.join("agent.sock"), tx: agent_tx, stop: None };
    agent.set(settings.claude.enabled);

    // After install: NSApp must be BaaApplication before gpui asks for it.
    let app = gpui_platform::application().with_assets(baa_ui::Assets);
    app.on_open_urls(move |urls| {
        for url in urls {
            tx.unbounded_send(url).ok();
        }
    });
    // A Dock click, or baa opened again, while no window shows.
    app.on_reopen(show);
    app.run(move |cx| {
        let host = core.start(cx);
        let agent = Rc::new(RefCell::new(agent));
        cx.set_global(baa_ui::AgentSwitch(Box::new(move |on| agent.borrow_mut().set(on))));
        launch(host, store, settings, dir, rx, agent_rx, cx);
    });
    std::process::exit(core.exit_code());
}

/// A running instance holds the lock; a second launch hands its URLs to it
/// and gets `None`.
fn single_instance(dir: &std::path::Path, urls: &[String]) -> Option<File> {
    let lock = File::create(dir.join("instance.lock")).unwrap_or_else(|e| fail("lock file", e));
    if lock.try_lock().is_err() {
        forward(dir, urls);
        return None;
    }
    Some(lock)
}

/// The rest of start-up, once the engine is up: key bindings, the window,
/// and links from other apps.
fn launch<H: WebHost>(
    host: H,
    store: Store,
    settings: Settings,
    dir: PathBuf,
    mut rx: mpsc::UnboundedReceiver<String>,
    mut agent_rx: mpsc::UnboundedReceiver<AgentPeer>,
    cx: &mut App,
) {
    baa_ui::init(cx);
    // Also tells the settings window where "Edit keymap.json…" goes.
    baa_ui::load_user_keymap(dir.join("keymap.json"), cx);
    cx.on_action(|_: &Quit, cx| quit::<H>(cx));
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
    menus::set(cx);
    // ponytail: gpui does not read the OS reduce-motion flag and we do not
    // either yet; Settings.reduce_motion is the only switch.

    let (display_id, bounds) = window_place(store.window_frame().ok().flatten(), cx);
    let downloads = host.downloads();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id,
        // The sidebar draws the toolbar; the traffic lights sit in it.
        titlebar: Some(TitlebarOptions {
            title: Some("baa".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(16.), px(12.))),
        }),
        // Pages are native views under gpui's layer, seen through the hole
        // each page element punches; that needs a transparent layer.
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let window: WindowHandle<Browser<H>> = cx
        .open_window(options, |window, cx| cx.new(|cx| Browser::new(host, store, window, cx)))
        .unwrap_or_else(|e| fail("opening the window", e));
    cx.activate(true);
    cx.set_global(Background { window: window.into(), downloads: downloads.clone(), quitting: false });
    window.update(cx, |_, window, cx| hide_when_last::<H>(window, cx)).ok();
    // Deferred: the new window is not in `cx.windows()` while its view is built.
    cx.observe_new::<Browser<H>>(|_, window, cx| {
        if let Some(window) = window {
            hide_when_last::<H>(window, cx);
        }
        cx.global_mut::<Background>().quitting = false;
    })
    .detach();
    cx.observe(&downloads, |_, cx| {
        if cx.global::<Background>().quitting && !downloading(cx) && nothing_shows(cx) {
            cx.quit();
        }
    })
    .detach();
    // The settings window cannot outlive the browser windows it belongs to.
    cx.on_window_closed(move |cx, _| {
        if !cx.windows().iter().any(|w| w.downcast::<Browser<H>>().is_some()) {
            cx.quit();
        }
        set_dock_menu::<H>(cx);
    })
    .detach();
    dock_menu::<H>(cx);

    // To look at the sidebar's update card: otherwise it shows only once an update is downloaded.
    if std::env::var_os("BAA_SHOW_UPDATE_CARD").is_some() {
        window.update(cx, |browser, _, cx| browser.update_ready(cx)).ok();
    }
    if let Some(updater::Started { ui, mut ready, mut install, mut checked }) = updater::start() {
        cx.set_global(ui);
        cx.spawn(async move |cx| {
            while let Some(outcome) = checked.next().await {
                cx.update(|cx| cx.set_global(outcome));
            }
        })
        .detach();
        cx.spawn(async move |cx| {
            while ready.next().await.is_some() {
                cx.update(|cx| {
                    for w in baa_ui::browser_windows::<H>(cx) {
                        w.update(cx, |browser, _, cx| browser.update_ready(cx)).ok();
                    }
                });
            }
        })
        .detach();
        // Here, between gpui's updates: installing quits the app.
        cx.spawn(async move |_| {
            while install.next().await.is_some() {
                updater::install();
            }
        })
        .detach();
    }

    cx.spawn(async move |cx| {
        while let Some(peer) = agent_rx.next().await {
            let Some(window) = cx.update(|cx| baa_ui::front_window::<H>(cx)) else { break };
            window.update(cx, |browser, window, cx| browser.serve_agent(peer, window, cx)).ok();
        }
    })
    .detach();

    // Links from other apps open in lil-baa, not the browser (Arc's Little Arc).
    cx.spawn(async move |cx| {
        while let Some(input) = rx.next().await {
            // ponytail: search engine as of launch; a later settings change
            // applies to forwarded URLs from the next launch.
            let url = settings.resolve_input(&input);
            let Some(window) = cx.update(|cx| baa_ui::front_window::<H>(cx)) else { break };
            window.update(cx, |browser, _, cx| browser.open_external(&url, cx)).ok();
        }
    })
    .detach();
}

/// Arc's dock menu in baa's terms; lil-baa is Little Arc. macOS adds the
/// window list, Options, Show All Windows, Hide and Quit itself.
fn dock_menu<H: WebHost>(cx: &mut App) {
    // A dock click usually comes while baa is inactive, so these arrive as global actions.
    // A new window calls off a quit that was waiting (see `observe_new` in `launch`).
    cx.on_action(|_: &DockNewWindow, cx| {
        cx.activate(true);
        baa_ui::open_window::<H>(false, cx);
    });
    cx.on_action(|_: &DockNewIncognito, cx| {
        cx.activate(true);
        baa_ui::open_window::<H>(true, cx);
    });
    cx.on_action(|_: &DockShowLil, cx| {
        cx.activate(true);
        for w in lil_windows::<H>(cx) {
            w.update(cx, |_, w, _| w.activate_window()).ok();
        }
    });
    cx.on_action(|_: &DockCloseLil, cx| {
        for w in lil_windows::<H>(cx) {
            w.update(cx, |_, w, _| w.remove_window()).ok();
        }
    });
    // Deferred: the new window is not in `cx.windows()` while its view is built.
    cx.observe_new::<LilBaa<H>>(|_, _, cx| cx.defer(set_dock_menu::<H>)).detach();
    set_dock_menu::<H>(cx);
}

fn lil_windows<H: WebHost>(cx: &App) -> Vec<gpui::AnyWindowHandle> {
    cx.windows().into_iter().filter(|w| w.downcast::<LilBaa<H>>().is_some()).collect()
}

/// Rebuilt whenever a window opens or closes, so the lil-baa items grey out
/// when there are none, as Arc's do.
fn set_dock_menu<H: WebHost>(cx: &mut App) {
    let none = lil_windows::<H>(cx).is_empty();
    cx.set_dock_menu(vec![
        MenuItem::action("New Window", DockNewWindow),
        MenuItem::action("New Incognito Window", DockNewIncognito),
        MenuItem::action("Show lil-baa Windows", DockShowLil).disabled(none),
        MenuItem::action("Close lil-baa Windows", DockCloseLil).disabled(none),
    ]);
}

/// Closing the last regular window is not quitting: it hides, baa stays in the Dock with
/// its pages and downloads, and a click there brings it back. With other windows open it
/// closes. A fullscreen window has its own desktop, which hiding the whole app leaves cleanly.
fn hide_when_last<H: WebHost>(window: &mut gpui::Window, cx: &mut App) {
    window.on_window_should_close(cx, |window, cx| {
        let this = window.window_handle().window_id();
        let incognito = !baa_ui::browser_windows::<H>(cx).iter().any(|w| w.window_id() == this);
        let others = baa_ui::browser_windows::<H>(cx).iter().any(|w| {
            w.window_id() != this && w.update(cx, |_, w, _| baa_web::window_shown(w)).unwrap_or(false)
        });
        if incognito || others {
            return true;
        }
        cx.global_mut::<Background>().window = window.window_handle();
        if window.is_fullscreen() {
            cx.hide();
        } else {
            baa_web::hide_window(window);
        }
        false
    });
}

/// What goes on while no window shows. Closing the browser window only hides it, and a
/// quit waits for the downloads under way: only a force quit cuts them off.
struct Background {
    window: AnyWindowHandle,
    downloads: Entity<Downloads>,
    /// ⌘Q came while files were coming in: baa quits when they are done, unless it is
    /// opened again first.
    quitting: bool,
}

impl gpui::Global for Background {}

/// A file is coming in. One that Chrome holds back for Keep or Discard is not: it waits
/// for the user, who has just left.
fn downloading(cx: &App) -> bool {
    coming_in(&cx.global::<Background>().downloads.read(cx).items)
}

fn coming_in(items: &[Download]) -> bool {
    items.iter().any(|d| d.state == DownloadState::InProgress && d.danger.is_none())
}

fn nothing_shows(cx: &mut App) -> bool {
    let main = cx.global::<Background>().window;
    cx.windows().len() == 1 && !main.update(cx, |_, window, _| baa_web::window_shown(window)).unwrap_or(false)
}

/// ⌘Q and the menu's Quit. Downloads under way finish first: the windows go, baa stays in
/// the Dock and quits when they are done.
// ponytail: the Dock's own Quit, a logout and "Restart and Update" end baa at once; they
// do not come through here. A download that stalls keeps baa running until it is opened
// again and the download cancelled; give the wait a limit if that bites.
fn quit<H: WebHost>(cx: &mut App) {
    if !cx.has_global::<Background>() || !downloading(cx) {
        return cx.quit();
    }
    cx.global_mut::<Background>().quitting = true;
    // The window that stays (hidden) is a regular one still open; the others close.
    if let Some(front) = baa_ui::front_window::<H>(cx) {
        cx.global_mut::<Background>().window = front.into();
    }
    let main = cx.global::<Background>().window;
    for handle in cx.windows() {
        handle
            .update(cx, |_, window, _| {
                if handle == main {
                    baa_web::hide_window(window);
                } else {
                    window.remove_window();
                }
            })
            .ok();
    }
}

/// Brings the browser window back, and calls off a quit that was waiting.
fn show(cx: &mut App) {
    if !cx.has_global::<Background>() {
        return;
    }
    cx.global_mut::<Background>().quitting = false;
    let main = cx.global::<Background>().window;
    cx.activate(true);
    main.update(cx, |_, window, _| window.activate_window()).ok();
}

/// Where the window opens: as it was last left, on the display it was on (the main one
/// if that is gone), or the usual size in the middle when nothing usable is saved.
fn window_place(saved: Option<WindowFrame>, cx: &App) -> (Option<DisplayId>, Bounds<Pixels>) {
    let usual = || (None, Bounds::centered(None, size(px(1280.), px(860.)), cx));
    let Some(saved) = saved else { return usual() };
    let named = |d: &Rc<dyn gpui::PlatformDisplay>| d.uuid().ok().map(|id| id.to_string()) == saved.display;
    let display = cx.displays().into_iter().find(|d| saved.display.is_some() && named(d)).or_else(|| cx.primary_display());
    let Some(display) = display else { return usual() };
    let area = display.bounds().size;
    if !frame_fits(saved.frame, (f32::from(area.width), f32::from(area.height))) {
        return usual();
    }
    let [x, y, w, h] = saved.frame;
    (Some(display.id()), Bounds::new(point(px(x), px(y)), size(px(w), px(h))))
}

/// The saved frame is a file's word: it is used only when it is a window that can be
/// seen and taken hold of on a display of this size (its top edge on it, 100 points of
/// its width).
fn frame_fits([x, y, w, h]: [f32; 4], (display_w, display_h): (f32, f32)) -> bool {
    [x, y, w, h].iter().all(|v| v.is_finite())
        && (400. ..=20_000.).contains(&w)
        && (300. ..=20_000.).contains(&h)
        && x + w >= 100.
        && x <= display_w - 100.
        && (0. ..=display_h - 100.).contains(&y)
}

fn fail(what: &str, err: impl std::fmt::Display) -> ! {
    eprintln!("baa: {what}: {err}");
    std::process::exit(1);
}

/// `<app data>/baa`: chromium/, session database, keymap.json and the instance lock.
/// A debug build is "baa Dev" (xtask gives its bundle its own id) so it runs beside an
/// installed baa instead of handing off to it. `BAA_DATA_DIR` replaces it, for tests.
fn data_dir() -> PathBuf {
    let var = |name| std::env::var_os(name).map(PathBuf::from);
    if let Some(dir) = var("BAA_DATA_DIR") {
        return dir;
    }
    let base = var("HOME").map(|h| h.join("Library/Application Support"));
    base.unwrap_or_else(std::env::temp_dir).join(if cfg!(debug_assertions) { "baa Dev" } else { "baa" })
}

// Open-URL IPC: one URL per line over a socket next to the lock. On macOS,
// LaunchServices (`open`, default-browser clicks) goes through on_open_urls
// instead; this covers launching the binary directly.
fn forward(dir: &std::path::Path, urls: &[String]) {
    use std::io::Write as _;
    match std::os::unix::net::UnixStream::connect(dir.join("ipc.sock")) {
        Ok(mut stream) => {
            for url in urls {
                let _ = writeln!(stream, "{url}");
            }
            eprintln!("baa is already running on {}; handed over {} URL(s). Quit it to start this copy.", dir.display(), urls.len());
        }
        Err(e) => eprintln!("baa is already running but did not answer: {e}"),
    }
}

fn listen(dir: &std::path::Path, tx: mpsc::UnboundedSender<String>) {
    use std::io::BufRead as _;
    let path = dir.join("ipc.sock");
    // A crashed run leaves the socket file behind; the lock proves nobody owns it.
    let _ = std::fs::remove_file(&path);
    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(e) => return log::error!("open-URL socket: {e}"),
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            for line in std::io::BufReader::new(stream)
                .lines()
                .map_while(Result::ok)
            {
                if !line.trim().is_empty() {
                    tx.unbounded_send(line).ok();
                }
            }
        }
    });
}

/// The uid on the other end of a unix socket.
fn peer_uid(stream: &UnixStream) -> Option<libc::uid_t> {
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: a valid fd for the call's duration, and two out-params it writes.
    (unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0).then_some(uid)
}

fn is_me(stream: &UnixStream) -> bool {
    // SAFETY: geteuid has no preconditions.
    peer_uid(stream) == Some(unsafe { libc::geteuid() })
}

/// The pid on the other end of a unix socket, as of its connect.
fn peer_pid(stream: &UnixStream) -> Option<libc::pid_t> {
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: a valid fd for the call's duration; the out-param is a pid_t and `len` says so.
    let ok = unsafe {
        libc::getsockopt(stream.as_raw_fd(), libc::SOL_LOCAL, libc::LOCAL_PEERPID, (&raw mut pid).cast(), &mut len)
    } == 0;
    (ok && pid > 0).then_some(pid)
}

fn pid_path(pid: libc::pid_t) -> Option<PathBuf> {
    // PROC_PIDPATHINFO_MAXSIZE.
    let mut buf = vec![0u8; 4 * libc::PATH_MAX as usize];
    // SAFETY: the buffer is as long as the size passed; proc_pidpath writes at most that.
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..n as usize])))
}

fn parent_pid(pid: libc::pid_t) -> Option<libc::pid_t> {
    // SAFETY: proc_bsdinfo is plain old data; all zeroes is a valid value.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: the buffer is a proc_bsdinfo and `size` is its size.
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (n == size).then_some(info.pbi_ppid as libc::pid_t).filter(|p| *p > 0)
}

/// The process the connect sheet names. A peer that is our own binary is a `baa mcp` bridge
/// whose client is its parent (Claude Code, or whatever ran it).
// ponytail: a pid can be reused between the connect and these lookups; the sheet informs, the
// token and the user's Allow are the gate.
fn agent_peer(stream: UnixStream) -> AgentPeer {
    let me = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    let mut pid = peer_pid(&stream);
    let mut path = pid.and_then(pid_path);
    if me.is_some() && path.as_ref().and_then(|p| p.canonicalize().ok()) == me {
        pid = pid.and_then(parent_pid);
        path = pid.and_then(pid_path);
    }
    AgentPeer { stream, pid, path: path.map(|p| p.display().to_string()) }
}

/// `baa mcp`: a hello line (BAA_AGENT_NONCE from the chat panel's `claude`, else the pairing
/// token from `claude mcp add -e BAA_AGENT_TOKEN=…`), then stdin to agent.sock and agent.sock to
/// stdout, until either side closes.
fn mcp_bridge(dir: &Path) {
    use std::io::Write as _;
    let var = |name| std::env::var(name).ok().filter(|v: &String| !v.is_empty());
    let (key, secret) = match (var("BAA_AGENT_NONCE"), var("BAA_AGENT_TOKEN")) {
        (Some(nonce), _) => ("nonce", nonce),
        (None, Some(token)) => ("token", token),
        (None, None) => fail("BAA_AGENT_TOKEN is not set", "copy the command from Settings → Claude again"),
    };
    // Both are hex, so the line below is valid JSON without an encoder.
    if !secret.bytes().all(|b| b.is_ascii_hexdigit()) {
        fail("BAA_AGENT_TOKEN", "copy the command from Settings → Claude again");
    }
    let hello = format!(r#"{{"baa":"hello","{key}":"{secret}"}}"#);
    let stream = UnixStream::connect(dir.join("agent.sock"))
        .unwrap_or_else(|e| fail("baa is not running, or Claude is off in Settings → Claude", e));
    // Talk only to our own app, whoever else could have put a socket there.
    if !is_me(&stream) {
        fail("agent.sock", "it belongs to another user");
    }
    if writeln!(&stream, "{hello}").is_err() {
        fail("agent.sock", "baa closed the connection");
    }
    let mut to_app = stream.try_clone().unwrap_or_else(|e| fail("agent.sock", e));
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut to_app);
        let _ = to_app.shutdown(std::net::Shutdown::Write);
    });
    let _ = std::io::copy(&mut &stream, &mut std::io::stdout().lock());
}

/// agent.sock (PLAN-m4-features D4): Claude's MCP clients, bound only while Settings → Claude
/// is on. Mode 0600 in the user's data dir; every connection's uid is checked as well, which
/// also covers one that races the chmod.
struct AgentSocket {
    path: PathBuf,
    tx: mpsc::UnboundedSender<AgentPeer>,
    /// Set while bound: tells the accept thread to stop.
    stop: Option<Arc<AtomicBool>>,
}

impl AgentSocket {
    fn set(&mut self, on: bool) {
        match (on, self.stop.take()) {
            (true, None) => self.stop = self.bind(),
            (true, stop) => self.stop = stop,
            (false, Some(stop)) => {
                stop.store(true, Ordering::SeqCst);
                // Wakes the blocked accept so the thread sees the flag and drops the listener.
                let _ = UnixStream::connect(&self.path);
                let _ = std::fs::remove_file(&self.path);
            }
            (false, None) => {}
        }
    }

    fn bind(&self) -> Option<Arc<AtomicBool>> {
        // A crashed run leaves the socket file behind; the instance lock proves nobody owns it.
        let _ = std::fs::remove_file(&self.path);
        let listener = UnixListener::bind(&self.path).map_err(|e| log::error!("agent socket: {e}")).ok()?;
        if let Err(e) = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600)) {
            log::error!("agent socket: {e}");
            let _ = std::fs::remove_file(&self.path);
            return None;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, tx) = (stop.clone(), self.tx.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                if !is_me(&stream) {
                    log::warn!("agent socket: refused a connection from another user");
                    continue;
                }
                if tx.unbounded_send(agent_peer(stream)).is_err() {
                    break;
                }
            }
        });
        Some(stop)
    }
}

#[cfg(test)]
mod tests {
    use super::{coming_in, frame_fits};
    use baa_web::{DangerKind, Download, DownloadState};

    #[test]
    fn a_quit_waits_only_for_files_that_are_coming_in() {
        let download = |state, danger| Download {
            id: 1,
            url: "https://a.example/f.zip".into(),
            path: "/tmp/f.zip".into(),
            received_bytes: 5,
            total_bytes: 10,
            state,
            ephemeral: false,
            danger,
        };
        assert!(!coming_in(&[]));
        assert!(coming_in(&[download(DownloadState::Complete, None), download(DownloadState::InProgress, None)]));
        assert!(!coming_in(&[download(DownloadState::Complete, None), download(DownloadState::Canceled, None)]));
        // Held back for Keep or Discard: nobody is there to answer, so it does not hold baa open.
        assert!(!coming_in(&[download(DownloadState::InProgress, Some(DangerKind::Dangerous))]));
    }

    #[test]
    fn a_saved_frame_is_used_only_where_it_can_be_seen() {
        let display = (1512., 982.);
        assert!(frame_fits([40., 60., 1400., 900.], display));
        assert!(frame_fits([-300., 25., 800., 600.], display), "partly off the left edge is still in reach");
        for bad in [
            [f32::NAN, 0., 800., 600.],
            [0., 0., 50., 600.],
            [0., 0., 800., 1e9],
            [3000., 60., 800., 600.],
            [-900., 60., 800., 600.],
            [40., -200., 800., 600.],
            [40., 950., 800., 600.],
        ] {
            assert!(!frame_fits(bad, display), "{bad:?}");
        }
    }
}
