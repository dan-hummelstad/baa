//! A verification code from mail, offered under a page's one-time-code field (PLAN-mail.md §7,
//! steps 2, 3 and 6; WP-L). The offer is a security prompt: the origin it names comes from the
//! engine, cut on the left; a click counts only after `ACTIVATION_DELAY` since it appeared or
//! changed; it never has the keyboard, so Enter and key repeat cannot accept it; its element ids
//! come from the field's id. A code is never logged, put in an id or a debug string, or kept.

use std::{
    collections::HashMap,
    time::{Duration, Instant, SystemTime},
};

use baa_mail::Code;
use baa_model::{Pane, SpaceId, TabId};
use baa_web::{PageEvent, PageHandle, WebHost};
use gpui::{div, point, prelude::*, px, AnyElement, Bounds, Context, Pixels};

use crate::{
    browser::{cut_left, host_of, plain_text, shadow, Browser},
    icons::icon,
    prompts::ACTIVATION_DELAY,
    theme::{ascii, r, surface},
};

/// How long a field is offered codes, and the earliest mail that counts.
const WINDOW: Duration = Duration::from_secs(120);
const BEFORE: Duration = Duration::from_secs(60);
/// At most one check of a Space's mail this often, whatever a page does.
const CHECK_EVERY: Duration = Duration::from_secs(10);
const OFFER_W: Pixels = px(300.);

/// No `Debug`: it holds a code.
pub(crate) struct Offer {
    tab: TabId,
    space: SpaceId,
    field: u64,
    /// The field's rectangle in the page.
    at: Bounds<Pixels>,
    /// The page's host, as the engine reported its origin.
    host: String,
    /// Mail from here on counts: the field's time less a minute.
    since: SystemTime,
    until: Instant,
    /// The code shown, and when it appeared or last changed.
    code: Option<(Code, Instant)>,
    /// Which offer this is, so a stale timer leaves a newer one alone.
    serial: u64,
}

#[derive(Default)]
pub(crate) struct Offers {
    pub(crate) offer: Option<Offer>,
    checked: HashMap<SpaceId, Instant>,
    serial: u64,
    #[cfg(test)]
    pub(crate) checks: u32,
}

fn host(origin: &str) -> Option<String> {
    let url = url::Url::parse(origin).ok()?;
    (url.scheme() == "https").then(|| url.host_str().map(str::to_owned)).flatten()
}

impl<H: WebHost> Browser<H> {
    pub(crate) fn on_code_field(&mut self, tab: TabId, event: &PageEvent, cx: &mut Context<Self>) {
        let PageEvent::CodeField { id, origin, top, at } = event else { return };
        let Some(at) = at else {
            if self.mail.as_ref().and_then(|m| m.offers.offer.as_ref()).is_some_and(|o| o.tab == tab && o.field == *id) {
                self.drop_offer(cx);
            }
            return;
        };
        let Some((space, host)) = self.may_offer(tab, origin, top) else { return };
        let now = cx.background_executor().now();
        let Some(mail) = &mut self.mail else { return };
        let offers = &mut mail.offers;
        offers.serial += 1;
        let serial = offers.serial;
        offers.offer = Some(Offer {
            tab,
            space,
            field: *id,
            at: *at,
            host,
            since: SystemTime::now() - BEFORE,
            until: now + WINDOW,
            code: None,
            serial,
        });
        if offers.checked.get(&space).is_none_or(|at| now.duration_since(*at) >= CHECK_EVERY) {
            offers.checked.insert(space, now);
            #[cfg(test)]
            {
                offers.checks += 1;
            }
            if let Some((mail, space)) = self.mail_of(space) {
                mail.check_now(space);
            }
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(WINDOW).await;
            let _ = this.update(cx, |this, cx| {
                if this.mail.as_ref().and_then(|m| m.offers.offer.as_ref()).is_some_and(|o| o.serial == serial) {
                    this.drop_offer(cx);
                }
            });
        })
        .detach();
        self.on_codes_changed(cx);
    }

    /// The Space and page host a field of `tab` may be offered codes in: the visible front tab
    /// of a Space that has mail (none incognito or in a private window), with codes on there and
    /// in settings, and a field in the top frame or one of its site.
    fn may_offer(&self, tab: TabId, origin: &str, top: &str) -> Option<(SpaceId, String)> {
        let space = self.session.tab(tab)?.space;
        if !self.settings.mail.codes || self.session.active_space() != Some(space) || self.front() != Some(Pane::Tab(tab)) {
            return None;
        }
        let (mail, mail_space) = self.mail_of(space)?;
        if !mail.accounts(mail_space).iter().any(|a| a.codes) {
            return None;
        }
        let (frame, page) = (host(origin)?, host(top)?);
        (frame == page || self.host.site_of(&frame) == self.host.site_of(&page)).then_some((space, page))
    }

    /// Mail changed, or the field just appeared: the newest code whose sender is of the page's site.
    pub(crate) fn on_codes_changed(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let Some(offer) = self.mail.as_ref().and_then(|m| m.offers.offer.as_ref()) else { return };
        if now >= offer.until {
            return self.drop_offer(cx);
        }
        let Some((mail, space)) = self.mail_of(offer.space) else { return self.drop_offer(cx) };
        let host = &self.host;
        let site = host.site_of(&offer.host);
        let code = mail.codes(space, offer.since).into_iter().find(|c| c.offered_on(&site, &|h| host.site_of(h)));
        let Some(offer) = self.mail.as_mut().and_then(|m| m.offers.offer.as_mut()) else { return };
        let same = offer.code.as_ref().map(|(c, _)| c) == code.as_ref();
        if !same {
            offer.code = code.map(|c| (c, now));
            cx.notify();
        }
    }

    pub(crate) fn drop_offer(&mut self, cx: &mut Context<Self>) {
        if let Some(mail) = &mut self.mail {
            if mail.offers.offer.take().is_some() {
                cx.notify();
            }
        }
    }

    /// After a session change: the offer goes once its tab is not the one in front.
    pub(crate) fn drop_hidden_offers(&mut self, cx: &mut Context<Self>) {
        let Some(offer) = self.mail.as_ref().and_then(|m| m.offers.offer.as_ref()) else { return };
        let (tab, space) = (offer.tab, offer.space);
        if self.session.active_space() != Some(space) || self.front() != Some(Pane::Tab(tab)) {
            self.drop_offer(cx);
        }
    }

    /// The tab navigated or closed.
    pub(crate) fn tab_left_page(&mut self, tab: TabId, cx: &mut Context<Self>) {
        if self.mail.as_ref().and_then(|m| m.offers.offer.as_ref()).is_some_and(|o| o.tab == tab) {
            self.drop_offer(cx);
        }
    }

    /// A click: the code goes into the field through the engine, which checks again that the
    /// field has focus and its origin is unchanged. Nothing submits the form.
    pub(crate) fn accept_offer(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let Some(offer) = self.mail.as_ref().and_then(|m| m.offers.offer.as_ref()) else { return };
        let Some((code, shown)) = &offer.code else { return };
        if now < *shown + ACTIVATION_DELAY {
            return;
        }
        if let Some((page, _)) = self.pages.get(&offer.tab) {
            page.read(cx).fill_code(offer.field, &code.code);
        }
        self.drop_offer(cx);
    }

    /// Under the field, in window coordinates, and across no other pane: the offer is the
    /// front tab's, which is the focused pane's page.
    pub(crate) fn render_code_offer(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let offer = self.mail.as_ref()?.offers.offer.as_ref()?;
        let (code, _) = offer.code.as_ref()?;
        let (mail, space) = self.mail_of(offer.space)?;
        let account = mail.accounts(space).into_iter().find(|a| a.id == code.account)?.address;
        let area = self.tab_area.get();
        let mut at = area.origin + offer.at.origin + point(px(0.), offer.at.size.height + px(4.));
        at.x = at.x.min(area.right() - OFFER_W).max(area.left());
        let field = offer.field;
        let ink = surface::ink();
        let line = |text: String| cut_left(div().min_w_0().text_size(px(11.)).text_color(surface::muted()).child(plain_text(&text)));
        Some(
            div()
                .id(("code-offer", field))
                .debug_selector(move || format!("code-offer-{field}"))
                .absolute()
                .left(at.x)
                .top(at.y)
                .w(OFFER_W)
                .occlude()
                .p(px(10.))
                .flex()
                .flex_col()
                .gap(px(3.))
                .bg(surface::bg())
                .border_1()
                .border_color(surface::border())
                .when(!ascii(), |d| d.rounded(r(10.)).shadow(vec![shadow(gpui::rgba(0x18122C29).into(), 6., 18.)]))
                .text_color(ink)
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.accept_offer(cx)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(icon("key-round", 14., surface::icon().into()))
                        .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(format!("Fill code {}", code.code))),
                )
                .child(line(format!("for {}", host_of(&format!("https://{}", offer.host)))))
                .child(line(format!("from {} · {account}", code.sender)))
                .into_any_element(),
        )
    }
}
