//! A small GET through a profile's network stack (its proxy settings and
//! request filter apply), for things like search suggestions.

use std::{cell::RefCell, rc::Rc};

use cef::*;

type Done = Box<dyn FnOnce(Option<String>)>;

wrap_urlrequest_client! {
    struct Fetch {
        body: Rc<RefCell<Vec<u8>>>,
        done: Rc<RefCell<Option<Done>>>,
    }

    impl UrlrequestClient {
        fn on_download_data(&self, _request: Option<&mut Urlrequest>, data: *const u8, data_length: usize) {
            if !data.is_null() {
                let chunk = unsafe { std::slice::from_raw_parts(data, data_length) };
                self.body.borrow_mut().extend_from_slice(chunk);
            }
        }

        fn on_request_complete(&self, request: Option<&mut Urlrequest>) {
            let ok = request.is_some_and(|r| r.request_status() == UrlrequestStatus::SUCCESS);
            let body = std::mem::take(&mut *self.body.borrow_mut());
            if let Some(done) = self.done.borrow_mut().take() {
                done(ok.then(|| String::from_utf8_lossy(&body).into_owned()));
            }
        }
    }
}

/// GETs `url` in `profile` (the global context if `None`); `done` gets the
/// body, or `None` on failure. Call on the main thread after `Runtime::start`.
pub fn fetch(profile: Option<&crate::Profile>, url: &str, done: impl FnOnce(Option<String>) + 'static) {
    let Some(mut request) = request_create() else {
        return done(None);
    };
    request.set_url(Some(&url.into()));
    request.set_method(Some(&"GET".into()));
    let done: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let mut client = Fetch::new(Rc::default(), done.clone());
    let mut context = profile.map(|p| p.context.clone());
    if urlrequest_create(Some(&mut request), Some(&mut client), context.as_mut()).is_none() {
        if let Some(done) = done.borrow_mut().take() {
            done(None);
        }
    }
}
