// What an extension sees of baa's tabs and windows (PLAN-chromium-phase3.md).
// Over DevTools: node cdp.mjs <port> 'probe()' chrome-extension
globalThis.probe = async () => ({
  tabs: (await chrome.tabs.query({})).map((t) => ({ id: t.id, windowId: t.windowId, active: t.active, url: t.url || t.pendingUrl })),
  windows: (await chrome.windows.getAll()).map((w) => ({ id: w.id, type: w.type, focused: w.focused, incognito: w.incognito })),
});
// An extension API call every 20 s keeps the worker (and its DevTools target) alive.
setInterval(() => chrome.runtime.getPlatformInfo(), 20000);
