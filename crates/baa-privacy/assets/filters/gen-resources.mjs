// Regenerates resources.json.gz (adblock-rust `Resource`s: uBO scriptlets + redirect bodies)
// from a uBlock Origin checkout. Mirrors uBO's redirect-engine.js loadBuiltinResources().
//   git clone --depth 1 https://github.com/gorhill/uBlock /tmp/uBlock
//   node gen-resources.mjs /tmp/uBlock | gzip -9 -n > resources.json.gz
// It imports uBO's scriptlet modules (it does not run any scriptlet).
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const ubo = process.argv[2];
const MIME = {
    js: 'application/javascript', css: 'text/css', html: 'text/html', json: 'application/json',
    gif: 'image/gif', png: 'image/png', mp3: 'audio/mp3', mp4: 'video/mp4', xml: 'text/xml',
};
const mime = name => name.endsWith('.fn') ? 'fn/javascript' : MIME[name.split('.').pop()] ?? 'text/plain';

const out = [];
const { builtinScriptlets } = await import(pathToFileURL(`${ubo}/src/js/resources/scriptlets.js`));
for (const s of builtinScriptlets) {
    out.push({
        name: s.name,
        aliases: s.aliases ?? [],
        kind: { mime: mime(s.name) },
        content: Buffer.from(s.fn.toString()).toString('base64'),
        dependencies: s.dependencies ?? [],
        // Bit 0 = TRUSTED in adblock_engine.rs: only uBO's own lists may use these.
        ...(s.requiresTrust ? { permission: 1 } : {}),
    });
}
const { default: war } = await import(pathToFileURL(`${ubo}/src/js/redirect-resources.js`));
for (const [name, details] of war) {
    out.push({
        name,
        aliases: details.alias === undefined ? [] : [].concat(details.alias),
        kind: { mime: mime(name) },
        content: readFileSync(`${ubo}/src/web_accessible_resources/${name}`).toString('base64'),
    });
}
process.stdout.write(JSON.stringify(out));
