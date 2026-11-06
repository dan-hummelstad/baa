//! baa docks DevTools itself, so the front-end needs its dock picker switched on.

/// What a console line starts with when it is a message for baa (`baa.h`).
const PREFIX: &str = "baa:";

/// DevTools' front-end shows its dock-side picker and docked close button only
/// when Chrome opened it with `can_dock=true`, which Chrome does only for
/// DevTools docked into its own window. baa docks it, so the first load comes
/// back with that set and `side` preselected; after that, a change in the
/// picker is reported as `devtools-dock:<side>` (or `undocked`).
///
/// Chrome is always told "not docked": `SetIsDocked(true)` would hit a DCHECK
/// in a window that cannot dock.
pub fn dock_script(url: &str, side: &str) -> String {
    let side = serde_json::to_string(side).unwrap_or_default();
    if !url.contains("can_dock=true") {
        return format!(
            r#"(() => {{
  InspectorFrontendHost.setPreference('currentDockState', JSON.stringify({side}));
  const url = new URL(location.href);
  url.searchParams.delete('dockSide');
  url.searchParams.set('can_dock', 'true');
  location.replace(url.href);
}})()"#
        );
    }
    format!(
        r#"(() => {{
  const host = InspectorFrontendHost;
  if (host.__appDock) return;
  host.__appDock = true;
  const setIsDocked = host.setIsDocked.bind(host);
  host.setIsDocked = (docked, done) => {{
    setIsDocked(false, done);
    host.getPreferences(prefs => {{
      let side = 'right';
      try {{ side = JSON.parse(prefs.currentDockState); }} catch {{}}
      console.debug({PREFIX:?} + 'devtools-dock:' + (docked ? side : 'undocked'));
    }});
  }};
}})()"#
    )
}
