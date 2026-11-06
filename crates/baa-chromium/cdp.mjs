// Evaluates a JS expression in the spike's page over DevTools and prints the
// result (the spike runs with --remote-debugging-port=<port>).
// With that switch and --no-startup-window, Chrome keeps itself alive until a
// DevTools client sends Browser.close (chrome_devtools_manager_delegate.cc),
// so such a run does not quit on ⌘Q; smoke.sh then kills it after 30 s.
//   node crates/baa-chromium/cdp.mjs <port> '<expression>'
const [port, expression] = process.argv.slice(2);
const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = targets.find((t) => t.url.startsWith('file:') || t.url.startsWith('http'));
if (!page) throw new Error('no page target');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve) => ws.addEventListener('open', resolve));
ws.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { expression, returnByValue: true, awaitPromise: true } }));
const reply = await new Promise((resolve) => ws.addEventListener('message', (m) => resolve(JSON.parse(m.data))));
console.log(JSON.stringify(reply.result?.result?.value ?? reply));
ws.close();
