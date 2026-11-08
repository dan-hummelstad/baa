//! Developer: DevTools, Chrome's pages, environments, the dev servers folder and console badges.

use baa_model::{DevToolsDock, Env, Project, Settings};
use baa_web::WebHost;
use gpui::{div, prelude::*, px, Context, SharedString};

use super::{Field, Item, SettingsView};
use crate::{
    icons::icon,
    theme::{r, surface},
};

impl<H: WebHost> SettingsView<H> {
    pub(super) fn developer(&self, s: &Settings, cx: &mut Context<Self>) -> Vec<Item> {
        const PAGES: [(&str, &str, &str); 7] = [
            ("page-flags", "Flags", "baa://flags"),
            ("page-about", "About", "baa://version"),
            ("page-certs", "Certificates", "baa://settings/certificates"),
            ("page-gpu", "GPU", "baa://gpu"),
            ("page-net", "Net internals", "baa://net-internals"),
            ("page-inspect", "Inspect devices", "baa://inspect"),
            ("page-all", "All Chrome pages", "baa://chrome-urls"),
        ];
        let pages = div().flex().flex_wrap().justify_end().gap(px(6.)).children(PAGES.into_iter().map(|(id, label, url)| {
            self.button(id, label, false, cx).on_click(cx.listener(move |this, _, _, cx| this.in_browser(cx, true, move |b, w, cx| b.open_url(url, w, cx))))
        }));
        let docks = [(DevToolsDock::Right, "Right"), (DevToolsDock::Bottom, "Bottom"), (DevToolsDock::Left, "Left"), (DevToolsDock::Window, "Window")];
        let docks = docks.iter().map(|(v, l)| (*v, SharedString::from(*l))).collect();
        let mut out = vec![
            self.section("Tools"),
            self.row(
                "devtools",
                "Where Inspect and ⌥⌘I open DevTools. DevTools' own Dock side menu changes it too.",
                self.segments("pref-devtools", docks, s.devtools_dock, |v, s| s.devtools_dock = v, cx),
            ),
            self.row("chrome_pages", "Chrome's own pages open in a tab.", pages.into_any_element()),
            self.section("Environments"),
        ];
        for (p, project) in s.environments.iter().enumerate() {
            for (e, env) in project.envs.iter().enumerate() {
                let remove = div()
                    .id(SharedString::from(format!("env-remove:{p}:{e}")))
                    .debug_selector(move || format!("env-remove:{p}:{e}"))
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(r(6.))
                    .hover(|d| d.bg(surface::key()))
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_env(p, e, cx)))
                    .child(icon("x", 13., surface::icon().into()))
                    .into_any_element();
                let text = format!("{}  ·  {}  {}", project.name, env.label, env.origin);
                out.push(self.sub_row(format!("env:{p}:{e}"), SharedString::from(text).into_any_element(), remove));
            }
        }
        let error = self.add_error.clone().filter(|_| !self.text(Field::EnvOrigin).is_empty());
        out.push(self.row(
            "environments",
            error.as_deref().unwrap_or("The header names the env a tab is on; ⌥⌘E moves it to the next, keeping the path."),
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(self.field(Field::EnvProject, "env-project", "Project", 80., cx))
                .child(self.field(Field::EnvLabel, "env-label", "Prod", 64., cx))
                .child(self.field(Field::EnvOrigin, "env-origin", "https://example.com", 150., cx))
                .child(self.button("env-add", "Add", true, cx).on_click(cx.listener(|this, _, _, cx| this.add_env(cx))))
                .into_any_element(),
        ));
        let on = self.browser.read(cx).has_dev_servers_folder();
        out.push(self.section("Local"));
        out.push(self.row(
            "dev_servers",
            "A live folder of the servers you run on localhost (node, vite, cargo…), also on the start page.",
            self.switch("pref-dev-servers", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.motion.from(("pref-dev-servers", 0), crate::motion::Feel::Glide, on as u8 as f32, !on as u8 as f32);
                    this.in_browser(cx, false, move |b, w, cx| b.set_dev_servers_folder(!on, w, cx));
                }))
                .into_any_element(),
        ));
        out.push(self.row(
            "console_badge",
            "A red count of console errors on localhost tabs; clicking it opens DevTools. Applies from a tab's next load.",
            self.toggle("pref-console-badge", s.console_badge, |s, v| s.console_badge = v, cx),
        ));
        out
    }

    /// The env from the three boxes; a project of the same name gets it added.
    pub(super) fn add_env(&mut self, cx: &mut Context<Self>) {
        let name = self.text(Field::EnvProject).trim().to_string();
        let label = self.text(Field::EnvLabel).trim().to_string();
        if name.is_empty() || label.is_empty() {
            return;
        }
        let Some(origin) = Env::origin_of(self.text(Field::EnvOrigin)) else {
            self.add_error = Some("The origin is an http(s) address with no path, like https://staging.example.com".into());
            return cx.notify();
        };
        for f in [Field::EnvLabel, Field::EnvOrigin] {
            self.field_mut(f).text.clear();
        }
        self.change(cx, |s| add_env(s, name, Env { label, origin }));
    }

    fn remove_env(&mut self, project: usize, env: usize, cx: &mut Context<Self>) {
        self.change(cx, |s| {
            let Some(p) = s.environments.get_mut(project) else { return };
            if env < p.envs.len() {
                p.envs.remove(env);
            }
            s.environments.retain(|p| !p.envs.is_empty());
        });
    }
}

/// Adds `env` to the project `name` (made if new); an env of the same label is replaced.
fn add_env(s: &mut Settings, name: String, env: Env) {
    let i = match s.environments.iter().position(|p| p.name.eq_ignore_ascii_case(&name)) {
        Some(i) => i,
        None => {
            s.environments.push(Project { name, envs: Vec::new() });
            s.environments.len() - 1
        }
    };
    let envs = &mut s.environments[i].envs;
    match envs.iter_mut().find(|e| e.label.eq_ignore_ascii_case(&env.label)) {
        Some(old) => *old = env,
        None => envs.push(env),
    }
}

#[cfg(test)]
#[test]
fn add_env_groups_by_project_and_replaces_by_label() {
    let mut s = Settings::default();
    let env = |label: &str, origin: &str| Env { label: label.into(), origin: origin.into() };
    add_env(&mut s, "shop".into(), env("Local", "http://localhost:3000"));
    add_env(&mut s, "Shop".into(), env("Prod", "https://shop.com"));
    add_env(&mut s, "shop".into(), env("local", "http://localhost:4000"));
    add_env(&mut s, "blog".into(), env("Prod", "https://blog.com"));
    assert_eq!(s.environments.len(), 2);
    assert_eq!(s.environments[0].envs, [env("local", "http://localhost:4000"), env("Prod", "https://shop.com")]);
}
