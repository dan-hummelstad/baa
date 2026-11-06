// Evaluates a JS expression in a page over DevTools and prints the result
// (the spike runs with --remote-debugging-port=<port>). `filter` picks the
// target whose URL contains it (default: the first file:/http page);
// "chrome-extension" picks the test extension's service worker.
// With that switch and --no-startup-window, Chrome keeps itself alive until a
// DevTools client sends Browser.close (chrome_devtools_manager_delegate.cc),
// so such a run does not quit on ⌘Q; smoke.sh then kills it after 30 s.
// Pass the expression "Browser.close" to end such a run.
//   node crates/baa-chromium/cdp.mjs <port> '<expression>' [filter]
const [port, expression, filter] = process.argv.slice(2);
const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
if (expression === 'targets') {
  console.log(targets.map((t) => `${t.type} ${t.url}`).join('\n'));
  process.exit(0);
}
let page;
if (expression === 'Browser.close') {
  const version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  page = { webSocketDebuggerUrl: version.webSocketDebuggerUrl };
} else {
  page = targets.find((t) => (filter ? t.url.includes(filter) : t.url.startsWith('file:') || t.url.startsWith('http')));
}
if (!page) throw new Error('no matching target');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve) => ws.addEventListener('open', resolve));
const method = expression === 'Browser.close' ? 'Browser.close' : 'Runtime.evaluate';
const params = method === 'Browser.close' ? {} : { expression, returnByValue: true, awaitPromise: true };
ws.send(JSON.stringify({ id: 1, method, params }));
// Events (worker targets send some first) have no id.
const reply = await new Promise((resolve) =>
  ws.addEventListener('message', (m) => {
    const data = JSON.parse(m.data);
    if (data.id === 1) resolve(data);
  }),
);
console.log(JSON.stringify(reply.result?.result?.value ?? reply));
ws.close();
