//! Inbox categories (PLAN-mail.md §17): the sender's rule, then the headers, then Apple's
//! on-device model for bulk mail. A category only chooses the pill a row shows under; it never
//! acts on a message.

use std::{
    sync::{atomic::Ordering, mpsc::Receiver, Arc, LazyLock, Mutex},
    time::Duration,
};

use crate::{store::Store, sync::Shared, AccountId, Event, Mail, MessageId, ThreadId};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Category {
    #[default]
    Primary,
    Social,
    Promotions,
}

impl Category {
    pub(crate) fn of(n: i64) -> Category {
        match n {
            1 => Category::Social,
            2 => Category::Promotions,
            _ => Category::Primary,
        }
    }

    pub(crate) fn n(self) -> i64 {
        self as i64
    }
}

/// Who set a message's category, as mail.db keeps it. NULL there: bulk mail waiting for the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum By {
    Rule = 0,
    Model = 1,
    User = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelStatus {
    Available,
    /// No Apple silicon, or Apple Intelligence is not offered in the Mac's region.
    NotEligible,
    /// macOS 26 only: on 27 it cannot be turned off.
    AppleIntelligenceOff,
    NotReady,
    /// Before macOS 26, a build without the bridge, or a reason this baa does not know.
    Unavailable,
}

pub(crate) enum Answer {
    Is(Category),
    /// The model failed on this message: it stays in Primary and is not asked again.
    Failed,
    /// Not now (rate-limited, not ready, Low Power Mode, a hot Mac): ask again in a minute.
    Wait,
}

pub(crate) type Model = Box<dyn Fn(&str) -> Answer + Send + Sync>;

#[derive(serde::Deserialize)]
struct Lists {
    bulk_headers: Vec<String>,
    social_domains: Vec<String>,
}

fn lists() -> &'static Lists {
    static LISTS: LazyLock<Lists> =
        LazyLock::new(|| serde_json::from_str(include_str!("categories.json")).expect("categories.json is valid"));
    &LISTS
}

pub(crate) fn domain(email: &str) -> String {
    email.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).unwrap_or_default()
}

pub(crate) fn rule_key(domain: &str) -> String {
    format!("category:{domain}")
}

fn within(domain: &str, of: &str) -> bool {
    domain == of || domain.strip_suffix(of).is_some_and(|rest| rest.ends_with('.'))
}

/// Steps 2 and 3 of §17 (the user's rule is step 1): `None` is bulk mail left to the model.
pub(crate) fn by_rules(header: &[u8]) -> Option<Category> {
    let Some(message) = mail_parser::MessageParser::new().parse_headers(header) else { return Some(Category::Primary) };
    let headers = message.headers();
    let value = |name: &str| headers.all(name).next().map(|h| String::from_utf8_lossy(h.raw_value()).trim().to_ascii_lowercase());
    let bulk = lists().bulk_headers.iter().any(|name| headers.all(name.as_str()).next().is_some())
        || value("Precedence").is_some_and(|p| matches!(p.as_str(), "bulk" | "list" | "junk"))
        || value("Auto-Submitted").is_some_and(|a| a != "no");
    if !bulk {
        return Some(Category::Primary);
    }
    let from = crate::mime::addresses(message.from()).into_iter().next().map(|a| domain(&a.email)).unwrap_or_default();
    // A discussion list gives an address to post to (RFC 2369); a newsletter does not.
    let discussion = value("List-Post").is_some_and(|p| p != "no");
    if discussion || lists().social_domains.iter().any(|d| within(&from, d)) {
        return Some(Category::Social);
    }
    None
}

/// What the model is shown: who, to whom, what about, which bulk headers, and the start of the
/// text. Capped so it stays far inside the model's 4,096-token context.
pub(crate) fn prompt(header: &[u8], snippet: &str) -> String {
    const MAX: usize = 2000;
    let e = crate::mime::envelope(header);
    let names = |list: &[crate::Address]| list.iter().map(|a| format!("{} <{}>", a.name, a.email)).collect::<Vec<_>>().join(", ");
    let bulk: Vec<&str> = match mail_parser::MessageParser::new().parse_headers(header) {
        Some(m) => lists().bulk_headers.iter().map(String::as_str).filter(|n| m.headers().all(*n).next().is_some()).collect(),
        None => Vec::new(),
    };
    let mut text = format!(
        "From: {} <{}>\nTo: {}\nSubject: {}\nBulk-mail headers: {}\n\n{}",
        e.from.name,
        e.from.email,
        names(&e.to),
        e.subject,
        if bulk.is_empty() { "none".into() } else { bulk.join(", ") },
        snippet.chars().take(1000).collect::<String>()
    );
    if text.len() > MAX {
        let mut end = MAX;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

impl Mail {
    /// Settings › Mail "Sort the inbox into categories". Off: the model is not asked; mail the
    /// rules sort still carries its category, and the pills are hidden.
    pub fn set_categories(&self, on: bool) {
        let was = self.shared.categories.swap(on, Ordering::Relaxed);
        if let (true, false, Some(store)) = (on, was, self.store.get()) {
            self.shared.categorize(store);
        }
    }

    /// The user moved a conversation to a pill: it, and all mail from its sender's domain, now
    /// and later, shows there. Private mode: a thread is of an account, which only a
    /// [`crate::MailSpace`] has.
    pub fn move_to_category(&self, thread: ThreadId, category: Category) {
        let Some(store) = self.store.get() else { return };
        for account in store.move_to_category(thread, category).unwrap_or_default() {
            (self.shared.notify)(Event::Changed { account });
        }
    }
}

impl Shared {
    pub(crate) fn categorize(self: &Arc<Self>, store: &Store) {
        if self.model.is_none() {
            return;
        }
        let wake = self.categorizer.get_or_init(|| {
            let (wake, rx) = std::sync::mpsc::channel();
            if let Ok(store) = store.another() {
                let shared = self.clone();
                let _ = std::thread::Builder::new().name("mail-categories".into()).spawn(move || run(store, &shared, rx));
            }
            Mutex::new(wake)
        });
        let _ = wake.lock().unwrap_or_else(|p| p.into_inner()).send(());
    }
}

/// The thread that asks the model, one for all of Mail. It sleeps until woken by new mail and
/// takes the newest waiting message first.
fn run(store: Store, shared: &Shared, wake: Receiver<()>) {
    let Some(model) = &shared.model else { return };
    const AGAIN: Duration = if cfg!(test) { Duration::from_millis(20) } else { Duration::from_secs(60) };
    loop {
        let next = if shared.categories.load(Ordering::Relaxed) { store.next_to_categorize().ok().flatten() } else { None };
        let Some((id, account, prompt)) = next else {
            if wake.recv().is_err() {
                return;
            }
            continue;
        };
        let category = match model(&prompt) {
            Answer::Is(category) => category,
            Answer::Failed => Category::Primary,
            Answer::Wait => {
                let _ = wake.recv_timeout(AGAIN);
                continue;
            }
        };
        if store.set_category(id, category, By::Model).is_ok() {
            (shared.notify)(Event::Changed { account });
        }
    }
}

pub(crate) type Waiting = (MessageId, AccountId, String);

#[cfg(baa_model)]
mod apple {
    use super::{Answer, Category, ModelStatus};

    extern "C" {
        fn baa_model_status() -> i32;
        fn baa_should_wait() -> i32;
        fn baa_categorize(prompt: *const std::ffi::c_char, timeout_ms: i32) -> i32;
    }

    pub(crate) fn status() -> ModelStatus {
        // SAFETY: no arguments; the bridge checks the OS version itself.
        match unsafe { baa_model_status() } {
            0 => ModelStatus::Available,
            1 => ModelStatus::NotEligible,
            2 => ModelStatus::AppleIntelligenceOff,
            3 => ModelStatus::NotReady,
            _ => ModelStatus::Unavailable,
        }
    }

    pub(crate) fn categorize(prompt: &str) -> Answer {
        // SAFETY: no arguments.
        if status() != ModelStatus::Available || unsafe { baa_should_wait() } != 0 {
            return Answer::Wait;
        }
        let Ok(prompt) = std::ffi::CString::new(prompt.replace('\0', "")) else { return Answer::Failed };
        // SAFETY: `prompt` is a valid C string that outlives the call, which copies it.
        match unsafe { baa_categorize(prompt.as_ptr(), 5000) } {
            0 => Answer::Is(Category::Primary),
            1 => Answer::Is(Category::Social),
            2 => Answer::Is(Category::Promotions),
            -2 => Answer::Wait,
            _ => Answer::Failed,
        }
    }
}

pub fn model_status() -> ModelStatus {
    #[cfg(baa_model)]
    return apple::status();
    #[cfg(not(baa_model))]
    ModelStatus::Unavailable
}

pub(crate) fn apple_model() -> Option<Model> {
    #[cfg(baa_model)]
    return Some(Box::new(apple::categorize));
    #[cfg(not(baa_model))]
    None
}

#[cfg(test)]
mod tests;
