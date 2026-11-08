//! The keymap's actions, bound to the browser window.

use super::*;

impl<H: WebHost> Browser<H> {
    pub(super) fn register_actions(&self, root: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        root.on_action(
            cx.listener(|this, _: &CommandBar, window, cx| match this.bar {
                Some(_) => this.close_overlays(window, cx),
                None => this.open_bar(Mode::NewTab, window, cx),
            }),
        )
        .on_action(cx.listener(|this, _: &FocusAddress, window, cx| {
            this.open_bar(Mode::Address, window, cx)
        }))
        .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
            this.sidebar_open = !this.sidebar_open;
            this.sidebar_w.set_target(if this.sidebar_open {
                this.sidebar_width()
            } else {
                0.0
            });
            this.reveal_hover = false;
            this.reveal.snap(0.0);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
            if let Some(id) = this.active_tab() {
                this.close_tab_id(id, window, cx);
            }
        }))
        .on_action(
            cx.listener(|this, _: &ReopenClosedTab, window, cx| this.reopen_closed(window, cx)),
        )
        .on_action(cx.listener(|this, _: &NextTab, window, cx| this.step_tab(1, window, cx)))
        .on_action(cx.listener(|this, _: &PrevTab, window, cx| this.step_tab(-1, window, cx)))
        .on_action(cx.listener(|this, GoToTab(n): &GoToTab, window, cx| {
            if let Some(id) = this.space_tab_ids().get(*n) {
                this.activate_tab(*id, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &LastTab, window, cx| {
            if let Some(id) = this.space_tab_ids().last() {
                this.activate_tab(*id, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &NextSpace, window, cx| this.step_space(1, window, cx)))
        .on_action(cx.listener(|this, _: &PrevSpace, window, cx| this.step_space(-1, window, cx)))
        .on_action(cx.listener(|this, GoToSpace(n): &GoToSpace, window, cx| {
            if let Some(id) = this.session.spaces().get(*n).map(|s| s.id) {
                this.activate_space(id, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &TogglePin, window, cx| {
            if let Some(id) = this.active_tab() {
                this.session.toggle_pin(id);
                this.sync(window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Back, _, cx| this.with_page(cx, |p| p.go_back())))
        .on_action(cx.listener(|this, _: &Forward, _, cx| this.with_page(cx, |p| p.go_forward())))
        .on_action(cx.listener(|this, _: &Reload, _, cx| {
            this.with_page(cx, |p| if p.is_loading() { p.stop() } else { p.reload(false) })
        }))
        .on_action(cx.listener(|this, _: &HardReload, _, cx| this.with_page(cx, |p| p.reload(true))))
        .on_action(cx.listener(|this, _: &CopyUrl, _, cx| this.copy_url(cx)))
        .on_action(cx.listener(|this, _: &Find, window, cx| {
            if this.active_page().is_some() {
                this.bar = None;
                this.find = Some(TextField::default());
                this.page_state.find_result = None;
                window.focus(&this.input_focus, cx);
                cx.notify();
            }
        }))
        .on_action(
            cx.listener(|this, _: &DevTools, window, cx| this.toggle_devtools(None, window, cx)),
        )
        .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_by(1, cx)))
        .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_by(-1, cx)))
        .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.zoom_by(0, cx)))
        .on_action(cx.listener(|this, _: &NewSpace, window, cx| this.new_space(window, cx)))
        .on_action(cx.listener(|this, _: &Print, _, cx| this.with_page(cx, |p| p.print())))
        .on_action(cx.listener(|this, _: &ViewSource, window, cx| this.view_source(window, cx)))
        .on_action(cx.listener(|this, _: &CycleEnvironment, window, cx| this.switch_env(None, window, cx)))
        .on_action(cx.listener(|this, _: &ReaderMode, _, cx| {
            this.with_page(cx, |p| p.eval(crate::tools::READER_SCRIPT))
        }))
        .on_action(cx.listener(|this, _: &PictureInPicture, _, cx| {
            this.with_page(cx, |p| p.eval_with_gesture(crate::tools::PIP_SCRIPT))
        }))
        .on_action(cx.listener(|this, _: &SplitView, window, cx| {
            if let Some(space) = this.session.active_space() {
                this.session.split_with_recent(space, now());
                this.sync(window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ResponsiveView, window, cx| this.responsive_view(window, cx)))
        .on_action(cx.listener(|this, _: &MuteTab, _, cx| {
            if let Some(id) = this.active_tab() {
                this.toggle_mute(id, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ShowDownloads, window, cx| {
            match this.library.as_ref().map(|l| l.section) {
                Some(crate::library::Section::Downloads) => this.close_library(window, cx),
                _ => this.open_library(crate::library::Section::Downloads, cx),
            }
        }))
        .on_action(cx.listener(|this, _: &ShowHistory, window, cx| this.open_bar(Mode::History, window, cx)))
        .on_action(cx.listener(|this, _: &ClearToday, window, cx| this.clear_today(window, cx)))
        .on_action(cx.listener(|this, _: &ShowArchive, window, cx| {
            match this.library.as_ref().map(|l| l.section) {
                Some(crate::library::Section::Archive) => this.close_library(window, cx),
                _ => this.open_library(crate::library::Section::Archive, cx),
            }
        }))
        .on_action(cx.listener(|this, _: &OpenSettings, _, cx| this.open_settings_at(None, None, cx)))
        .on_action(cx.listener(|this, _: &NewIncognitoSpace, window, cx| {
            this.new_incognito_space(window, cx)
        }))
    }
}
