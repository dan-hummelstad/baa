//! The element a page renders as, and the mouse it forwards.

use std::rc::Rc;

use gpui::{
    relative, App, Bounds, DispatchPhase, Element, ElementId, FocusHandle, GlobalElementId, Hitbox, HitboxBehavior,
    InspectorElementId, IntoElement, LayoutId, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollWheelEvent,
    Style, Window,
};

use super::{native, Shared};

/// Where the page goes: moves the native view to the laid-out rect and makes
/// gpui's layer transparent over it.
pub(super) struct PageSlot {
    pub(super) shared: Rc<Shared>,
    pub(super) container: Rc<native::Container>,
    pub(super) focus_handle: FocusHandle,
    pub(super) corner_radius: f32,
    pub(super) square_corners: u8,
}

impl IntoElement for PageSlot {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for PageSlot {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> Hitbox {
        let visible = bounds.intersect(&window.content_mask().bounds);
        if visible.size.width > Pixels::ZERO && visible.size.height > Pixels::ZERO {
            if self.container.place(self.shared.view.get(), bounds, visible) {
                window.request_animation_frame();
            }
        } else {
            self.container.set_hidden(true);
        }
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        _: &mut App,
    ) {
        let radius = (self.corner_radius * window.scale_factor()).round() as usize;
        if let Some(hole) = native::punch_buffer(radius, self.square_corners) {
            window.paint_surface(bounds, hole);
        }

        // gpui hit-tests (overlays drawn over the page keep the mouse); the
        // page gets the original NSEvent, so Chromium sees real clicks,
        // trackpad phases and momentum. A drag stays with the view it started on.
        let (shared, hit, focus) = (self.shared.clone(), hitbox.clone(), self.focus_handle.clone());
        window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hit.is_hovered(window) {
                shared.drag.set(native::forward_current_event(shared.view.get(), None));
                window.focus(&focus, cx);
                shared.run(|core, tab| (core.tab_focus)(tab));
            }
        });
        let (shared, hit) = (self.shared.clone(), hitbox.clone());
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if let Some(view) = shared.drag.get().filter(|_| event.pressed_button.is_some()) {
                native::forward_current_event(shared.view.get(), Some(view));
                return;
            }
            // Moves over the page, and the first one off it so hover ends.
            let over = hit.is_hovered(window);
            if over || shared.inside.get() {
                shared.inside.set(over);
                native::forward_current_event(shared.view.get(), None);
            }
        });
        let shared = self.shared.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, _| {
            if phase == DispatchPhase::Bubble {
                if let Some(view) = shared.drag.take() {
                    native::forward_current_event(shared.view.get(), Some(view));
                }
            }
        });
        let (shared, hit) = (self.shared.clone(), hitbox.clone());
        window.on_mouse_event(move |_: &ScrollWheelEvent, phase, window, _| {
            if phase == DispatchPhase::Bubble && hit.should_handle_scroll(window) {
                native::forward_current_event(shared.view.get(), None);
            }
        });
    }
}
