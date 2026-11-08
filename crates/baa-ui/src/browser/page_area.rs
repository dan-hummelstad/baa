//! The page card with DevTools beside it, the collapsed sidebar's reveal, and the frame clock.

use super::*;

impl<H: WebHost> Browser<H> {
    pub(super) fn render_page(
        &mut self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let page = self.active_page().cloned();
        // The native view rounds itself; keep it on the card's radius as the style changes.
        if let Some(page) = &page {
            page.read(cx).set_corner_radius(f32::from(r(PAGE_RADIUS)));
        }
        let content = match page {
            Some(page) => page.update(cx, |p, cx| p.render(window, cx)),
            None => crate::start::render(theme).into_any_element(),
        };
        // Only the active tab's DevTools shows; the rest keep their state hidden.
        let active = self.active_tab();
        for (tab, (tools, _)) in &self.devtools {
            tools.read(cx).set_visible(Some(*tab) == active);
        }
        let dock = self.settings.devtools_dock;
        let tools = active.and_then(|tab| self.devtools.get(&tab)).map(|(tools, _)| tools.clone());
        // As in Chrome: one card, the two meeting at a straight seam.
        let (page_square, tools_square) = match (&tools, dock) {
            (None, _) | (_, DevToolsDock::Window) => (0, 0),
            (Some(_), DevToolsDock::Right) => (2 | 8, 1 | 4),
            (Some(_), DevToolsDock::Left) => (1 | 4, 2 | 8),
            (Some(_), DevToolsDock::Bottom) => (4 | 8, 1 | 2),
        };
        if let Some(page) = self.active_page() {
            page.read(cx).set_square_corners(page_square);
        }
        let across = dock == DevToolsDock::Bottom;
        let share = self.devtools_share;
        let tools = tools.map(|tools| {
            tools.read(cx).set_corner_radius(f32::from(r(PAGE_RADIUS)));
            tools.read(cx).set_square_corners(tools_square);
            let pane = div()
                .flex_none()
                .min_w_0()
                .min_h_0()
                .child(tools.update(cx, |p, cx| p.render(window, cx)));
            if across { pane.w_full().h(relative(share)) } else { pane.h_full().w(relative(share)) }
        });
        let resize = if across { gpui::CursorStyle::ResizeUpDown } else { gpui::CursorStyle::ResizeLeftRight };
        // The seam: a hairline, with a wider grip to drag it by.
        let seam = tools.is_some().then(|| {
            let grip = div()
                .id("devtools-seam")
                .absolute()
                .occlude()
                .cursor(resize)
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.devtools_drag = true;
                    cx.notify();
                }));
            let grip = if across { grip.left_0().right_0().top(px(-3.)).h(px(7.)) } else { grip.top_0().bottom_0().left(px(-3.)).w(px(7.)) };
            let line = div().flex_none().relative().bg(gpui::rgba(0x0000002E)).child(grip);
            if across { line.w_full().h(px(1.)) } else { line.h_full().w(px(1.)) }
        });
        let area = self.page_area.clone();
        let measure = gpui::canvas(move |bounds, _, _| area.set(bounds), |_, _, _, _| {}).absolute().size_full();
        // While dragging, the whole area follows the pointer (pages are native
        // views and would take the moves for themselves).
        let drag = self.devtools_drag.then(|| {
            let area = self.page_area.clone();
            div()
                .id("devtools-drag")
                .absolute()
                .inset_0()
                .occlude()
                .cursor(resize)
                .on_mouse_move(cx.listener(move |this, e: &gpui::MouseMoveEvent, _, cx| {
                    let b = area.get();
                    let share = match this.settings.devtools_dock {
                        DevToolsDock::Right => (b.right() - e.position.x) / b.size.width,
                        DevToolsDock::Left => (e.position.x - b.left()) / b.size.width,
                        _ => (b.bottom() - e.position.y) / b.size.height,
                    };
                    if share.is_finite() {
                        this.devtools_share = share.clamp(0.15, 0.85);
                        cx.notify();
                    }
                }))
                .on_mouse_up(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.devtools_drag = false;
                    cx.notify();
                }))
        });
        let asks = self.render_asks(theme, cx);
        let crashed = self.crash_overlay(cx);
        let notice = self.exclusive_notice();
        let count = self.page_state.find_result;
        let find = self.find.as_ref().map(|field| {
            drop_in(div()
                .id("find")
                // Pages are native views: only occluding gpui keeps the click.
                .occlude()
                .absolute()
                .top(px(12.))
                .right(px(12.))
                .w(px(280.))
                .h(px(40.))
                .px(px(14.))
                .flex()
                .items_center()
                .rounded(r(10.))
                .bg(surface::bg())
                .border_1()
                .border_color(surface::border())
                .shadow(vec![shadow(gpui::rgba(0x18122C29).into(), 8., 24.)])
                .text_size(px(14.))
                .track_focus(&self.input_focus)
                .on_key_down(cx.listener(Self::on_input_key))
                .child(field.render(
                    "Find in page",
                    surface::ink().into(),
                    surface::muted().into(),
                    Hsla { a: 0.25, ..theme.accent },
                ))
                .child(div().ml_auto().pl(px(8.)).text_size(px(12.)).text_color(match count {
                    Some((_, 0)) | None => surface::muted(),
                    Some(_) => surface::ink(),
                }).children(count.map(|(a, t)| format!("{a}/{t}")))), "find")
        });
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .when(across, |d| d.flex_col())
                    .when(dock == DevToolsDock::Left, |d| d.flex_row_reverse())
                    .rounded(r(PAGE_RADIUS))
                    .overflow_hidden()
                    .bg(gpui::white())
                    .shadow(vec![shadow(Hsla { a: 0.12, ..theme.ink }, 1., 4.)])
                    .child(measure)
                    .child(div().flex_1().min_h_0().min_w_0().child(content))
                    .children(seam)
                    .children(tools)
                    .children(drag),
            )
            // ASCII Chrome: the page sits in a box-drawn frame, a few px off its edge.
            .when(crate::theme::ascii(), |d| {
                d.p(px(4.)).border_1().border_color(crate::theme::ascii::frame()).rounded(px(5.))
            })
            .children(crashed)
            .children(asks)
            .children(find)
            .children(notice)
    }

    /// DevTools for the active tab where the settings put it; docked ones close
    /// when asked again, as ⌥⌘I does in Chrome.
    pub(crate) fn toggle_devtools(&mut self, inspect_at: Option<(i32, i32)>, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(tab), Some(page)) = (self.active_tab(), self.active_page().cloned()) else { return };
        let Some(side) = self.settings.devtools_dock.side() else {
            return page.read(cx).open_devtools(inspect_at);
        };
        if self.devtools.remove(&tab).is_some() {
            self.focus_page(window, cx);
            return cx.notify();
        }
        let tools = self.host.create_devtools(&page, inspect_at, side, window, cx);
        let sub = cx.subscribe_in(&tools, window, move |this, _, event, window, cx| match event {
            // Its own close button.
            PageEvent::Closed => {
                this.devtools.remove(&tab);
                this.focus_page(window, cx);
                cx.notify();
            }
            // Its own dock-side picker.
            PageEvent::DevToolsDock(side) => this.set_devtools_dock(DevToolsDock::from_side(side), window, cx),
            _ => {}
        });
        window.focus(&tools.read(cx).focus_handle(cx), cx);
        self.devtools.insert(tab, (tools, sub));
        cx.notify();
    }

    /// Moves DevTools to `dock` and remembers it; the active tab's follows.
    pub(crate) fn set_devtools_dock(&mut self, dock: baa_model::DevToolsDock, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.devtools_dock = dock;
        if let Some(Err(e)) = self.store.as_ref().map(|s| s.save_settings(&self.settings)) {
            log::error!("saving settings: {e}");
        }
        // A docked one just moves; one going to a window reopens there.
        if dock == DevToolsDock::Window && self.active_tab().and_then(|tab| self.devtools.remove(&tab)).is_some() {
            self.toggle_devtools(None, window, cx);
        }
        cx.notify();
    }

    /// The collapsed sidebar follows the pointer's position, not hover: the
    /// edge zone opens it, and it stays while the pointer is anywhere between
    /// the window edge and a little past the card (the inset gap included), or
    /// has left the window over that edge, or a menu from it is open.
    pub(super) fn track_reveal(&mut self, at: Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        if self.sidebar_open {
            return;
        }
        let x = f32::from(match self.side() {
            SidebarSide::Left => at.x,
            SidebarSide::Right => window.viewport_size().width - at.x,
        });
        let reach = if self.reveal_hover {
            INSET + self.reveal_w.value() + REVEAL_SLACK
        } else {
            REVEAL_EDGE
        };
        let inside = x < reach || (self.reveal_hover && self.menu_open);
        if inside != self.reveal_hover {
            self.set_reveal(inside, cx);
        }
    }

    pub(super) fn set_reveal(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.reveal_hover = hovered;
        if self.reveal_hovered() {
            self.reveal.set_target(1.0);
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REVEAL_LINGER).await;
            this.update(cx, |this, cx| {
                if !this.reveal_hovered() {
                    this.reveal.set_target(0.0);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The collapsed sidebar: an edge hot zone, and the sidebar as a floating card
    /// sliding in over the page (which keeps its size).
    pub(super) fn render_reveal(
        &mut self,
        sidebar: Option<AnyElement>,
        theme: &Theme,
    ) -> [AnyElement; 2] {
        let right = self.side() == SidebarSide::Right;
        let edge = |d: gpui::Div, offset: f32| {
            if right {
                d.right(px(offset))
            } else {
                d.left(px(offset))
            }
        };
        // Only keeps the edge's pointer events from the page; `track_reveal` decides.
        let zone = edge(div().absolute().top_0().bottom_0().w(px(REVEAL_EDGE)), 0.)
            .id("reveal-edge")
            .occlude();
        let shown = self.reveal.value();
        // Off-screen at 0, inset at 1; the spring's overshoot reads as Arc's bounce.
        let width = self.reveal_w.value();
        let offset = INSET + (shown - 1.0) * (width + INSET * 2.0);
        let card = sidebar.map(|sidebar| {
            edge(div().absolute().top(px(HEADER_H)).bottom(px(INSET)), offset)
                .id("reveal-card")
                .occlude()
                .w(px(width))
                // The page's radius: a larger one shows the page past its corners.
                .rounded(r(PAGE_RADIUS))
                .overflow_hidden()
                .bg(gradient_fill(theme))
                .border_1()
                .border_color(Hsla { a: 0.10, ..theme.ink })
                .shadow(vec![
                    shadow(Hsla { a: 0.20, ..theme.ink }, 12., 32.),
                    shadow(Hsla { a: 0.10, ..theme.ink }, 2., 6.),
                ])
                .child(sidebar)
                .into_any_element()
        });
        [
            zone.into_any_element(),
            card.unwrap_or_else(|| div().into_any_element()),
        ]
    }

    pub(crate) fn reveal_hovered(&self) -> bool {
        self.reveal_hover
    }

    pub(super) fn tick_motion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = cx.background_executor().now(); // the fake clock under test, so motion is deterministic there
        // Capped: the first frame after an idle stretch would otherwise carry the
        // whole idle time and finish every spring in one step (no animation).
        let dt = (now - self.last_frame).as_secs_f32().min(1. / 30.);
        self.last_frame = now;
        let reduce = cx.reduce_motion();
        let mut moving = false;
        self.reveal_w.set_target(self.sidebar_width());
        // Retargeted before the tick, so a key press that moves the selection asks for a frame.
        if let Some(bar) = &self.bar {
            self.motion.to(("bar-sel", 0), Feel::Glide, bar.selected as f32);
        }
        for spring in [
            &mut self.sidebar_w,
            &mut self.reveal_w,
            &mut self.reveal,
            &mut self.slide,
            &mut self.fade,
        ] {
            moving |= spring.tick(dt, reduce);
        }
        if let Some(peek) = &mut self.peek {
            moving |= peek.open.tick(dt, reduce);
        }
        if let Some(bar) = &mut self.bar {
            moving |= bar.scale.tick(dt, reduce);
        }
        if let Some(bar) = &mut self.closing_bar {
            let more = bar.scale.tick(dt, reduce);
            moving |= more;
            if !more {
                self.closing_bar = None;
            }
        }
        moving |= self.motion.tick(dt, reduce);
        let motion = &self.motion;
        self.ghosts.retain(|g| motion.get(("ghost", g.id.0), Feel::Glide) > 0.0);
        self.folder_motion.retain(|_, spring| {
            let more = spring.tick(dt, reduce);
            moving |= more;
            more
        });
        if moving {
            window.request_animation_frame();
        }
    }

    /// Rows that appear in a list grow in; rows that leave one (closed, pinned,
    /// filed, archived) collapse where they were.
    pub(super) fn track_rows(&mut self) {
        let s = &self.session;
        let mut now = HashMap::new();
        let mut add = |list: List, tabs: Vec<&Tab>| {
            for (index, t) in tabs.into_iter().enumerate() {
                now.insert(
                    t.id,
                    Placement {
                        list,
                        index,
                        title: tab_title(t).to_string(),
                        favicon: t.favicon.clone(),
                    },
                );
            }
        };
        for space in s.spaces() {
            for f in s.folders(space.id) {
                add((space.id, Zone::Pinned, Some(f.id)), s.folder_tabs(f.id).collect());
            }
            add((space.id, Zone::Pinned, None), s.tabs(space.id, Zone::Pinned).collect());
            add((space.id, Zone::Today, None), s.tabs(space.id, Zone::Today).collect());
        }
        if let Some(first) = s.spaces().first() {
            add(FAVORITES, s.tabs(first.id, Zone::Favorite).collect());
        }
        let Some(before) = self.placements.replace(now) else {
            return;
        };
        let now = self.placements.as_ref().expect("just set");
        for (id, was) in &before {
            if now.get(id).is_some_and(|p| p.list == was.list) || was.list == FAVORITES {
                continue;
            }
            self.ghosts.retain(|g| g.id != *id);
            self.ghosts.push(Ghost {
                id: *id,
                list: was.list,
                index: was.index,
                title: was.title.clone().into(),
                favicon: was.favicon.clone(),
            });
            self.motion.from(("ghost", id.0), Feel::Glide, 1.0, 0.0);
        }
        for (id, p) in now {
            if before.get(id).is_none_or(|was| was.list != p.list) {
                self.motion.from(("tab-in", id.0), Feel::Glide, 1.0, 0.0);
            }
        }
    }
}
