//! The CEF-side callbacks.
//!
//! cef-rs's `wrap_*!` macros define a handler as "a struct with fields, plus
//! overrides of the trait's default methods". The fields have to be `Clone`, so
//! shared state is passed around as `Rc<Shared>`.

use std::{path::PathBuf, rc::Rc};

use cef::{rc::Rc as _, *};
use core_foundation::base::TCFType;
use core_video::pixel_buffer::CVPixelBuffer;
use gpui::{px, size, Bounds, CursorStyle, RenderImage};

use super::shared::{Frame as SharedFrame, Shared};
use crate::{Download, DownloadState, FilterHook, WebviewEvent};

/// Settings carried by `cef::App`.
#[derive(Clone)]
pub(crate) struct AppConfig {
    pub(crate) remote_debugging_port: Option<u16>,
    pub(crate) log_to_stderr: bool,
    pub(crate) switches: Vec<(String, Option<String>)>,
}

wrap_app! {
    pub(crate) struct GpuiCefApp {
        config: AppConfig,
    }

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            let Some(command_line) = command_line else {
                return;
            };

            // Off-screen rendering: keep CEF from creating windows of its own.
            command_line.append_switch(Some(&"no-startup-window".into()));
            command_line.append_switch(Some(&"noerrdialogs".into()));
            command_line.append_switch(Some(&"hide-crash-restore-bubble".into()));
            // Avoids the macOS Keychain prompt. Only meaningful in development.
            command_line.append_switch(Some(&"use-mock-keychain".into()));

            // An escape hatch for when the child processes will not start. Not
            // for everyday use: turning the sandbox off removes Chromium's
            // process isolation.
            if std::env::var("GPUI_CEF_NO_SANDBOX").is_ok() {
                command_line.append_switch(Some(&"no-sandbox".into()));
            }

            if self.config.log_to_stderr {
                command_line.append_switch(Some(&"enable-logging=stderr".into()));
            }

            if let Some(port) = self.config.remote_debugging_port {
                command_line.append_switch_with_value(
                    Some(&"remote-debugging-port".into()),
                    Some(&port.to_string().as_str().into()),
                );
            }

            for (name, value) in &self.config.switches {
                let key = CefString::from(name.as_str());
                match value {
                    Some(value) => {
                        // Appending replaces; feature lists have to be merged.
                        let existing = CefString::from(&command_line.switch_value(Some(&key))).to_string();
                        let value = if name.ends_with("-features") && !existing.is_empty() {
                            format!("{existing},{value}")
                        } else {
                            value.clone()
                        };
                        command_line.append_switch_with_value(Some(&key), Some(&value.as_str().into()));
                    }
                    None => command_line.append_switch(Some(&key)),
                }
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(GpuiCefBrowserProcessHandler::new())
        }
    }
}

wrap_browser_process_handler! {
    pub(crate) struct GpuiCefBrowserProcessHandler;

    impl BrowserProcessHandler {
        /// With `external_message_pump` enabled, CEF uses this to say when it
        /// would like `do_message_loop_work()` to be called. Waiting for the next
        /// vsync tick instead costs a tick per browser-thread hop, and a frame
        /// takes several: 40 fps measured instead of 120. Any thread.
        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            super::pump::schedule_work(delay_ms);
        }
    }
}

// Only stashes frames in `Shared`; gpui does the drawing.
wrap_render_handler! {
    pub(crate) struct GpuiCefRenderHandler {
        shared: Rc<Shared>,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            let Some(rect) = rect else { return };
            let size = self.shared.view_size();
            // CEF does not accept a zero-sized view.
            rect.x = 0;
            rect.y = 0;
            rect.width = (f32::from(size.width).round() as i32).max(1);
            rect.height = (f32::from(size.height).round() as i32).max(1);
        }

        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            screen_info: Option<&mut ScreenInfo>,
        ) -> ::std::os::raw::c_int {
            let Some(screen_info) = screen_info else {
                return 0;
            };
            screen_info.device_scale_factor = self.shared.scale_factor();
            1
        }

        /// Where the text being composed sits on screen. macOS needs this to put
        /// the IME candidate window next to the caret rather than in the corner.
        fn on_ime_composition_range_changed(
            &self,
            _browser: Option<&mut Browser>,
            selected_range: Option<&Range>,
            character_bounds: Option<&[Rect]>,
        ) {
            super::input_trace(format_args!(
                "CEF composition range changed selected={selected_range:?} bounds={}",
                character_bounds.map_or(0, <[Rect]>::len)
            ));
            let bounds = character_bounds
                .unwrap_or_default()
                .iter()
                .map(|rect| Bounds {
                    origin: gpui::point(px(rect.x as f32), px(rect.y as f32)),
                    size: size(px(rect.width as f32), px(rect.height as f32)),
                })
                .collect();
            self.shared.set_composition_bounds(bounds);
        }

        /// Whether the popup layer — a `<select>` dropdown and the like — is on
        /// screen. CEF draws it as a separate layer, so it has to be composited
        /// over the page rather than ignored.
        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: ::std::os::raw::c_int) {
            self.shared.set_popup_visible(show != 0);
        }

        /// Where the popup goes, in the webview's own coordinate space.
        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) {
            let Some(rect) = rect else { return };
            self.shared.set_popup_rect(Bounds {
                origin: gpui::point(px(rect.x as f32), px(rect.y as f32)),
                size: size(px(rect.width as f32), px(rect.height as f32)),
            });
        }

        /// The GPU path: take the shared IOSurface CEF drew into and wrap it in a
        /// `CVPixelBuffer`, ready for gpui's `surface()`. No pixels are copied.
        fn on_accelerated_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            info: Option<&AcceleratedPaintInfo>,
        ) {
            let Some(info) = info else { return };
            if info.shared_texture_io_surface.is_null() {
                return;
            }

            // The IOSurface CEF hands over is only guaranteed for the duration of
            // the call, but it is refcounted, so retaining it under the get rule
            // keeps it alive.
            //
            // core-video's CVPixelBuffer::from_io_surface takes the io-surface
            // crate's type, so the deprecation warning is tolerated right here
            // (there is no path through objc2-io-surface).
            #[allow(deprecated)]
            let surface = unsafe {
                io_surface::IOSurface::wrap_under_get_rule(
                    info.shared_texture_io_surface as io_surface::IOSurfaceRef,
                )
            };

            match CVPixelBuffer::from_io_surface(&surface, None) {
                Ok(buffer) => {
                    let popup = type_ == PaintElementType::POPUP;
                    if let Some(owned) = self.shared.copy_accelerated(popup, &buffer) {
                        put_frame(&self.shared, type_, SharedFrame::Accelerated(owned));
                    }
                }
                Err(status) => {
                    log::error!("CVPixelBufferCreateWithIOSurface failed: {status}");
                }
            }
        }

        /// The CPU path, for when shared textures are unavailable. Copies BGRA
        /// pixels every frame, so it is slower than the accelerated one.
        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            if buffer.is_null() || width <= 0 || height <= 0 {
                return;
            }

            let len = width as usize * height as usize * 4;
            let pixels = unsafe { std::slice::from_raw_parts(buffer, len) }.to_vec();

            // gpui's textures expect BGRA, which is exactly what CEF produces.
            let Some(image) = image::ImageBuffer::from_raw(width as u32, height as u32, pixels)
            else {
                return;
            };
            let frame = SharedFrame::Cpu(std::sync::Arc::new(RenderImage::new(vec![
                image::Frame::new(image),
            ])));
            put_frame(&self.shared, type_, frame);
        }
    }
}

fn put_frame(shared: &Shared, kind: PaintElementType, frame: SharedFrame) {
    if kind == PaintElementType::POPUP {
        shared.put_popup_frame(frame);
    } else {
        shared.put_frame(frame);
    }
}

wrap_display_handler! {
    pub(crate) struct GpuiCefDisplayHandler {
        shared: Rc<Shared>,
    }

    impl DisplayHandler {
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            if let Some(title) = title {
                self.shared.set_title(title.to_string());
            }
        }

        fn on_address_change(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            url: Option<&CefString>,
        ) {
            // Subframes report their own addresses too; the page's is the main one.
            if frame.is_some_and(|frame| frame.is_main() == 0) {
                return;
            }
            if let Some(url) = url {
                self.shared.set_url(url.to_string());
            }
        }

        fn on_favicon_urlchange(
            &self,
            _browser: Option<&mut Browser>,
            icon_urls: Option<&mut CefStringList>,
        ) {
            // Taking the list moves CEF's borrowed one out; the empty one left
            // behind is ours to free.
            let first = icon_urls.and_then(|urls| std::mem::take(urls).into_iter().next());
            self.shared.push_event(WebviewEvent::FaviconChanged(first));
        }

        /// A console line carrying [`crate::MESSAGE_PREFIX`] is a message for
        /// the application: it is queued for the pump and kept off the console.
        fn on_console_message(
            &self,
            _browser: Option<&mut Browser>,
            _level: cef::LogSeverity,
            message: Option<&CefString>,
            _source: Option<&CefString>,
            _line: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let Some(message) = message.map(|message| message.to_string()) else {
                return 0;
            };
            match crate::message_payload(&message) {
                Some(payload) => {
                    self.shared.push_event(WebviewEvent::Message(payload.to_string()));
                    1
                }
                None => 0,
            }
        }

        /// Off-screen rendering means CEF never touches the real cursor, so the
        /// page's choice is forwarded to gpui instead.
        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            _cursor: *mut u8,
            type_: CursorType,
            _custom_cursor_info: Option<&CursorInfo>,
        ) -> ::std::os::raw::c_int {
            self.shared.set_cursor(cursor_style(type_));
            // Returning 1 tells CEF the cursor has been dealt with.
            1
        }
    }
}

/// Maps CEF's cursor kinds onto the closest thing gpui can show.
fn cursor_style(cursor: CursorType) -> CursorStyle {
    match cursor {
        CursorType::HAND => CursorStyle::PointingHand,
        CursorType::IBEAM => CursorStyle::IBeam,
        CursorType::VERTICALTEXT => CursorStyle::IBeamCursorForVerticalLayout,
        CursorType::CROSS | CursorType::CELL => CursorStyle::Crosshair,
        CursorType::MOVE | CursorType::MIDDLEPANNING => CursorStyle::ClosedHand,
        CursorType::GRAB => CursorStyle::OpenHand,
        CursorType::GRABBING => CursorStyle::ClosedHand,
        CursorType::CONTEXTMENU => CursorStyle::ContextualMenu,
        CursorType::ALIAS | CursorType::DND_LINK => CursorStyle::DragLink,
        CursorType::COPY | CursorType::DND_COPY => CursorStyle::DragCopy,
        CursorType::NOTALLOWED | CursorType::NODROP | CursorType::DND_NONE => {
            CursorStyle::OperationNotAllowed
        }
        // gpui has no hidden cursor; the arrow is the least surprising stand-in.
        CursorType::NONE => CursorStyle::Arrow,
        CursorType::EASTRESIZE | CursorType::WESTRESIZE => CursorStyle::ResizeLeftRight,
        CursorType::NORTHRESIZE | CursorType::SOUTHRESIZE => CursorStyle::ResizeUpDown,
        CursorType::EASTWESTRESIZE => CursorStyle::ResizeLeftRight,
        CursorType::NORTHSOUTHRESIZE => CursorStyle::ResizeUpDown,
        CursorType::NORTHEASTRESIZE | CursorType::SOUTHWESTRESIZE => {
            CursorStyle::ResizeUpRightDownLeft
        }
        CursorType::NORTHWESTRESIZE | CursorType::SOUTHEASTRESIZE => {
            CursorStyle::ResizeUpLeftDownRight
        }
        CursorType::NORTHEASTSOUTHWESTRESIZE => CursorStyle::ResizeUpRightDownLeft,
        CursorType::NORTHWESTSOUTHEASTRESIZE => CursorStyle::ResizeUpLeftDownRight,
        CursorType::COLUMNRESIZE => CursorStyle::ResizeColumn,
        CursorType::ROWRESIZE => CursorStyle::ResizeRow,
        // WAIT, PROGRESS, HELP, ZOOMIN/OUT and the panning cursors have no gpui
        // equivalent; the arrow is the least surprising fallback.
        _ => CursorStyle::Arrow,
    }
}

wrap_load_handler! {
    pub(crate) struct GpuiCefLoadHandler {
        shared: Rc<Shared>,
    }

    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            _browser: Option<&mut Browser>,
            is_loading: ::std::os::raw::c_int,
            can_go_back: ::std::os::raw::c_int,
            can_go_forward: ::std::os::raw::c_int,
        ) {
            self.shared.set_load_state(
                is_loading != 0,
                can_go_back != 0,
                can_go_forward != 0,
            );
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            // ERR_ABORTED just means another navigation replaced this one.
            if error_code == Errorcode::from(cef::sys::cef_errorcode_t::ERR_ABORTED) {
                return;
            }
            log::warn!(
                "failed to load {}: {} ({:?})",
                failed_url.map(|url| url.to_string()).unwrap_or_default(),
                error_text.map(|text| text.to_string()).unwrap_or_default(),
                error_code,
            );
        }
    }
}

// Window-opening requests become [`WebviewEvent::OpenRequested`]; the app
// decides where they go.
wrap_life_span_handler! {
    pub(crate) struct GpuiCefLifeSpanHandler {
        shared: Rc<Shared>,
    }

    impl LifeSpanHandler {
        // ponytail: the popup is cancelled, so `window.opener` is null in the new
        // tab and opener-based OAuth flows break. Ceiling: needs a way to hand a
        // CEF-created popup browser to the app as a page.
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            request_open(&self.shared, target_url, user_gesture)
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            if let Some(browser) = browser {
                self.shared.set_browser(browser.clone());
            }
        }

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            self.shared.push_event(WebviewEvent::Closed);
        }
    }
}

// Always 1: the engine never opens the window itself.
fn request_open(
    shared: &Shared,
    url: Option<&CefString>,
    user_gesture: ::std::os::raw::c_int,
) -> ::std::os::raw::c_int {
    if let Some(url) = url {
        shared.push_event(WebviewEvent::OpenRequested {
            url: url.to_string(),
            from_user_gesture: user_gesture != 0,
        });
    }
    1
}

wrap_request_handler! {
    pub(crate) struct GpuiCefRequestHandler {
        shared: Rc<Shared>,
    }

    impl RequestHandler {
        /// Cmd/Ctrl/middle-click on a link.
        fn on_open_urlfrom_tab(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            target_url: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            user_gesture: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            if target_disposition == WindowOpenDisposition::CURRENT_TAB {
                return 0;
            }
            request_open(&self.shared, target_url, user_gesture)
        }
    }
}

// Off-screen browsers cannot show a native menu; the app draws its own.
wrap_context_menu_handler! {
    pub(crate) struct GpuiCefContextMenuHandler {
        shared: Rc<Shared>,
    }

    impl ContextMenuHandler {
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
        ) {
            if let Some(model) = model {
                model.clear();
            }
            let Some(params) = params else { return };
            let non_empty = |s: CefStringUserfree| {
                Some(CefString::from(&s).to_string()).filter(|s| !s.is_empty())
            };
            let origin = self.shared.bounds().origin;
            self.shared.push_event(WebviewEvent::ContextMenu {
                x: f32::from(origin.x) + params.xcoord() as f32,
                y: f32::from(origin.y) + params.ycoord() as f32,
                link_url: non_empty(params.link_url()),
                selection: non_empty(params.selection_text()),
            });
        }
    }
}

wrap_download_handler! {
    pub(crate) struct GpuiCefDownloadHandler {
        shared: Rc<Shared>,
    }

    impl DownloadHandler {
        // cef-rs defaults every hook to 0, which here would cancel all downloads.
        fn can_download(
            &self,
            _browser: Option<&mut Browser>,
            _url: Option<&CefString>,
            _request_method: Option<&CefString>,
        ) -> ::std::os::raw::c_int {
            1
        }

        fn on_before_download(
            &self,
            _browser: Option<&mut Browser>,
            _download_item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> ::std::os::raw::c_int {
            let Some(callback) = callback else { return 0 };
            let name = suggested_name.map(|name| name.to_string()).unwrap_or_default();
            let path = unique_path(&downloads_dir(), &name);
            callback.cont(Some(&path.to_string_lossy().as_ref().into()), 0);
            1
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut Browser>,
            item: Option<&mut DownloadItem>,
            _callback: Option<&mut DownloadItemCallback>,
        ) {
            let Some(item) = item else { return };
            let state = if item.is_complete() != 0 {
                DownloadState::Complete
            } else if item.is_canceled() != 0 || item.is_interrupted() != 0 {
                DownloadState::Canceled
            } else {
                DownloadState::InProgress
            };
            self.shared.push_event(WebviewEvent::Download(Download {
                id: item.id(),
                url: CefString::from(&item.url()).to_string(),
                path: CefString::from(&item.full_path()).to_string(),
                received_bytes: item.received_bytes(),
                total_bytes: item.total_bytes(),
                state,
            }));
        }
    }
}

// ponytail: $HOME/Downloads, no user setting. Ceiling: a download-folder pref.
fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    home.map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Downloads")
}

/// `dir/name`, or `dir/name (n).ext` for the first `n` that is free. Only the
/// last component of `name` is used, so a page cannot pick another directory.
fn unique_path(dir: &std::path::Path, name: &str) -> PathBuf {
    let name = std::path::Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("download");
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    (0..)
        .map(|n| match n {
            0 => dir.join(name),
            n => dir.join(format!("{stem} ({n}){ext}")),
        })
        .find(|path| !path.exists())
        .expect("unbounded range")
}

// The per-profile network filter. Lives on the request context, so it also
// sees requests no browser made (service workers). IO thread.
wrap_request_context_handler! {
    pub(crate) struct ProfileContextHandler {
        filter: FilterHook,
        prefs_off: &'static [&'static str],
    }

    impl RequestContextHandler {
        // UI thread; prefs cannot be set before this.
        fn on_request_context_initialized(&self, request_context: Option<&mut RequestContext>) {
            let Some(context) = request_context else { return };
            for name in self.prefs_off {
                let Some(mut value) = value_create() else { continue };
                value.set_bool(0);
                let mut error = CefString::default();
                if context.set_preference(Some(&(*name).into()), Some(&mut value), Some(&mut error)) == 0 {
                    log::info!("pref {name} not set: {error}");
                }
            }
        }

        fn resource_request_handler(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            request: Option<&mut cef::Request>,
            _is_navigation: ::std::os::raw::c_int,
            _is_download: ::std::os::raw::c_int,
            request_initiator: Option<&CefString>,
            _disable_default_handling: Option<&mut ::std::os::raw::c_int>,
        ) -> Option<ResourceRequestHandler> {
            let request = request?;
            let url = CefString::from(&request.url()).to_string();
            let kind = resource_type(request.resource_type());
            let initiator = request_initiator.map(|i| i.to_string()).unwrap_or_default();
            let page_initiated = frame.is_some() || !initiator.is_empty();
            let source_url = match kind {
                baa_privacy::ResourceType::MainFrame => url.clone(),
                // A subframe's own URL is not loaded yet; its parent is asking.
                baa_privacy::ResourceType::SubFrame => frame
                    .and_then(|f| f.parent())
                    .map(|p| CefString::from(&p.url()).to_string())
                    .unwrap_or(initiator),
                _ => frame
                    .map(|f| CefString::from(&f.url()).to_string())
                    .filter(|u| !u.is_empty())
                    .unwrap_or(initiator),
            };
            let block = (self.filter)(&crate::Request {
                url: &url,
                source_url: &source_url,
                resource_type: kind,
                page_initiated,
            });
            // None means CEF's default handling, so allowed requests cost nothing.
            block.then(CancelHandler::new)
        }
    }
}

wrap_resource_request_handler! {
    pub(crate) struct CancelHandler {}

    impl ResourceRequestHandler {
        // cef-rs's default here is RV_CANCEL already, but be explicit.
        fn on_before_resource_load(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            _request: Option<&mut cef::Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            ReturnValue::CANCEL
        }
    }
}

fn resource_type(kind: ResourceType) -> baa_privacy::ResourceType {
    use baa_privacy::ResourceType as R;
    match kind {
        ResourceType::MAIN_FRAME => R::MainFrame,
        ResourceType::SUB_FRAME => R::SubFrame,
        ResourceType::STYLESHEET => R::Stylesheet,
        ResourceType::SCRIPT => R::Script,
        ResourceType::IMAGE => R::Image,
        ResourceType::FONT_RESOURCE => R::Font,
        ResourceType::OBJECT => R::Object,
        ResourceType::MEDIA => R::Media,
        ResourceType::WORKER => R::Worker,
        ResourceType::SHARED_WORKER => R::SharedWorker,
        ResourceType::PREFETCH => R::Prefetch,
        ResourceType::FAVICON => R::Favicon,
        ResourceType::XHR => R::Xhr,
        ResourceType::PING => R::Ping,
        ResourceType::SERVICE_WORKER => R::ServiceWorker,
        ResourceType::CSP_REPORT => R::CspReport,
        ResourceType::PLUGIN_RESOURCE => R::PluginResource,
        ResourceType::NAVIGATION_PRELOAD_MAIN_FRAME => R::NavigationPreloadMainFrame,
        ResourceType::NAVIGATION_PRELOAD_SUB_FRAME => R::NavigationPreloadSubFrame,
        _ => R::SubResource,
    }
}

wrap_client! {
    pub(crate) struct GpuiCefClient {
        render_handler: RenderHandler,
        display_handler: DisplayHandler,
        load_handler: LoadHandler,
        life_span_handler: LifeSpanHandler,
        request_handler: RequestHandler,
        context_menu_handler: ContextMenuHandler,
        download_handler: DownloadHandler,
    }

    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(self.life_span_handler.clone())
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(self.request_handler.clone())
        }

        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            Some(self.context_menu_handler.clone())
        }

        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(self.download_handler.clone())
        }

        fn render_handler(&self) -> Option<RenderHandler> {
            Some(self.render_handler.clone())
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(self.display_handler.clone())
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(self.load_handler.clone())
        }
    }
}

impl GpuiCefClient {
    pub(crate) fn build(shared: Rc<Shared>) -> Client {
        Self::new(
            GpuiCefRenderHandler::new(shared.clone()),
            GpuiCefDisplayHandler::new(shared.clone()),
            GpuiCefLoadHandler::new(shared.clone()),
            GpuiCefLifeSpanHandler::new(shared.clone()),
            GpuiCefRequestHandler::new(shared.clone()),
            GpuiCefContextMenuHandler::new(shared.clone()),
            GpuiCefDownloadHandler::new(shared),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_cursors_are_mapped() {
        assert_eq!(cursor_style(CursorType::HAND), CursorStyle::PointingHand);
        assert_eq!(cursor_style(CursorType::IBEAM), CursorStyle::IBeam);
        assert_eq!(cursor_style(CursorType::POINTER), CursorStyle::Arrow);
    }

    #[test]
    fn unmapped_cursors_fall_back_to_the_arrow() {
        // No gpui equivalent, and guessing would be worse than an arrow.
        assert_eq!(cursor_style(CursorType::WAIT), CursorStyle::Arrow);
        assert_eq!(cursor_style(CursorType::ZOOMIN), CursorStyle::Arrow);
    }

    #[test]
    fn opposite_resize_edges_share_a_cursor() {
        // Left and right edges look the same, as they do everywhere else on macOS.
        assert_eq!(
            cursor_style(CursorType::EASTRESIZE),
            cursor_style(CursorType::WESTRESIZE)
        );
        assert_eq!(
            cursor_style(CursorType::NORTHRESIZE),
            cursor_style(CursorType::SOUTHRESIZE)
        );
    }

    #[test]
    fn download_names_never_escape_or_overwrite() {
        let dir = std::env::temp_dir().join(format!("gpui-cef-dl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "../../evil.sh"), dir.join("evil.sh"));
        assert_eq!(unique_path(&dir, ""), dir.join("download"));

        std::fs::write(dir.join("a.tar.gz"), b"").unwrap();
        assert_eq!(unique_path(&dir, "a.tar.gz"), dir.join("a.tar (1).gz"));
        std::fs::write(dir.join("README"), b"").unwrap();
        assert_eq!(unique_path(&dir, "README"), dir.join("README (1)"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
