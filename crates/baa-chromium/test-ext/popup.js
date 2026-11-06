// The popup reports the tab it acts on: with real tabs it is the active tab of the
// current window, no shim needed (PLAN-chromium-phase4.md step 6).
chrome.tabs.query({ active: true, currentWindow: true }).then((tabs) => {
  document.title = 'active:' + (tabs[0] && tabs[0].url);
  document.body.textContent = document.title;
});
