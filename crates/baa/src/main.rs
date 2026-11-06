//! App shell: wires baa-web's browser engine (CEF, or BaaCore with the `core`
//! feature), baa-model's store and baa-ui's root view into one window, and
//! keeps one running instance per profile (PLAN.md §6).

use std::{fs::File, path::PathBuf};

use baa_model::{Settings, Store};
use baa_ui::Browser;
use baa_web::WebHost;
#[cfg(not(feature = "core"))]
use baa_web::CefRuntime;
use futures::{channel::mpsc, StreamExt as _};
use gpui::{
    actions, point, px, size, App, AppContext as _, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowOptions,
};

actions!(baa_app, [Quit]);

#[cfg(not(feature = "core"))]
fn main() {
    env_logger::init();
    let dir = data_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        fail("creating the data directory", e);
    }
    // CEF switches (e.g. the CI test's --proxy-server) start with '-'; the rest are URLs.
    let urls: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();

    // Held until the process exits.
    let Some(_lock) = single_instance(&dir, &urls) else { return };

    let cef = CefRuntime::init(dir.join("profile")).unwrap_or_else(|e| fail("loading CEF", e));
    let store = Store::open(dir.join("baa.db")).unwrap_or_else(|e| fail("opening baa.db", e));
    let settings = store.load_settings().unwrap_or_default();

    let (tx, rx) = mpsc::unbounded::<String>();
    for url in urls {
        tx.unbounded_send(url).ok();
    }
    listen(&dir, tx.clone());

    let app = gpui_platform::application().with_assets(baa_ui::Assets);
    app.on_open_urls(move |urls| {
        for url in urls {
            tx.unbounded_send(url).ok();
        }
    });
    let runtime = cef.clone();
    app.run(move |cx| {
        // gpui has created NSApp by now; CEF must initialise after it or it
        // claims the NSApplication singleton and gpui crashes.
        let host = runtime
            .start(cx)
            .unwrap_or_else(|e| fail("starting CEF", e));
        launch(host, store, settings, dir, rx, cx);
    });

    cef.shutdown();
}

/// BaaCore: Chromium's own main runs inside gpui's `[NSApp run]`, so the
/// order here is load-bearing (PLAN-chromium-phase2.md "Results").
#[cfg(feature = "core")]
fn main() {
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

    // After install: NSApp must be BaaApplication before gpui asks for it.
    let app = gpui_platform::application().with_assets(baa_ui::Assets);
    app.on_open_urls(move |urls| {
        for url in urls {
            tx.unbounded_send(url).ok();
        }
    });
    app.run(move |cx| {
        let host = core.start(cx);
        launch(host, store, settings, dir, rx, cx);
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
    cx: &mut App,
) {
    baa_ui::init(cx);
    if let Ok(json) = std::fs::read_to_string(dir.join("keymap.json")) {
        if let Err(e) = baa_ui::actions::bind_keymap(&json, cx) {
            log::error!("{e:#}");
        }
    }
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
    cx.set_menus([Menu::new("baa").items([MenuItem::action("Quit baa", Quit)])]);
    // ponytail: gpui does not read the OS reduce-motion flag and we do not
    // either yet; Settings.reduce_motion is the only switch.
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();

    let bounds = Bounds::centered(None, size(px(1280.), px(860.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
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

    // Links from other apps open in lil-baa, not the browser (Arc's Little Arc).
    cx.spawn(async move |cx| {
        while let Some(input) = rx.next().await {
            // ponytail: search engine as of launch; a later settings change
            // applies to forwarded URLs from the next launch.
            let url = settings.resolve_input(&input);
            let opened = window.update(cx, |browser, _, cx| browser.open_external(&url, cx));
            if opened.is_err() {
                break;
            }
        }
    })
    .detach();
}

fn fail(what: &str, err: impl std::fmt::Display) -> ! {
    eprintln!("baa: {what}: {err}");
    std::process::exit(1);
}

/// `<app data>/baa`: profile, session database, keymap.json and the instance lock.
/// `BAA_DATA_DIR` replaces it, for tests.
fn data_dir() -> PathBuf {
    let var = |name| std::env::var_os(name).map(PathBuf::from);
    if let Some(dir) = var("BAA_DATA_DIR") {
        return dir;
    }
    let base = if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library/Application Support"))
    } else if cfg!(windows) {
        var("APPDATA")
    } else {
        var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("baa")
}

// Open-URL IPC: one URL per line over a socket next to the lock. On macOS,
// LaunchServices (`open`, default-browser clicks) goes through on_open_urls
// instead; this covers launching the binary directly.
#[cfg(unix)]
fn forward(dir: &std::path::Path, urls: &[String]) {
    use std::io::Write as _;
    match std::os::unix::net::UnixStream::connect(dir.join("ipc.sock")) {
        Ok(mut stream) => {
            for url in urls {
                let _ = writeln!(stream, "{url}");
            }
        }
        Err(e) => eprintln!("baa is already running but did not answer: {e}"),
    }
}

#[cfg(unix)]
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

// ponytail: no named pipe on Windows yet (not buildable here, and gpui-cef's
// Windows backend is still WebView2). A second launch only explains and exits.
#[cfg(not(unix))]
fn forward(_: &std::path::Path, _: &[String]) {
    eprintln!("baa is already running");
}

#[cfg(not(unix))]
fn listen(_: &std::path::Path, _: mpsc::UnboundedSender<String>) {}
