// Reconstructed from supplied HTML VB-40, audited 7fe1b032dff5143df41b1df22ecaf23c4e2f7cb6.
// Exercises candidate's actual policy builder; original PoC bundle unavailable.
// Candidate SHA/build artifact digests and browser evidence are recorded by qualification.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { integrity, secureHtml, webPolicy } from '../web-policy.ts';

const env = {
  VITE_FUNDS_ORIGIN: 'https://funds.example.test',
  VITE_FAUCET_ORIGIN: 'https://faucet.example.test',
  BRIDGE_CSP_CONNECT_ORIGINS: '["https://rpc.example.test","wss://rpc.example.test"]',
  VITE_ISOLATED_FAUCET: 'true',
};
const html =
  '<html><head><link rel="stylesheet" crossorigin href="/assets/main.css"></head><body><script type="module" crossorigin src="/assets/main.js"></script></body></html>';
const assets = () =>
  new Map([
    ['assets/main.js', 'console.log("qualified entry");'],
    ['assets/main.css', 'body{color:white}'],
  ]);

await test('HTML7fe1b032 funds entry is hash-authorized with exact connections and SRI', () => {
  const resources = assets();
  const secured = secureHtml(html, resources, webPolicy(env, false));
  const hash = integrity(resources.get('assets/main.js')!);
  assert(secured.html.includes(`integrity="${hash}" crossorigin="anonymous"`));
  assert(secured.html.includes(`integrity="${integrity(resources.get('assets/main.css')!)}"`));
  const script = secured.csp.split('; ').find((directive) => directive.startsWith('script-src '))!;
  assert.equal(script, `script-src '${hash}'`);
  assert(!secured.csp.includes('challenges.cloudflare.com'));
  for (const directive of [
    "default-src 'none'",
    "worker-src 'none'",
    "object-src 'none'",
    "base-uri 'none'",
    "frame-ancestors 'none'",
    "form-action 'none'",
  ])
    assert(secured.csp.includes(directive));
  assert(secured.csp.includes("connect-src 'self' https://rpc.example.test wss://rpc.example.test"));
  resources.set('assets/main.js', 'console.log("tampered entry");');
  assert.notEqual(integrity(resources.get('assets/main.js')!), hash, 'tampering cannot satisfy emitted SRI/CSP hash');
});

await test('HTML7fe1b032 rejects permissive origins and unprotected executable surfaces', () => {
  for (const origin of [
    'https:',
    'https://*',
    'https://rpc.test/path',
    'https://u:p@rpc.test',
    'https://rpc.test?q=1',
    "https://rpc.test 'unsafe-eval'",
    'http://rpc.test',
  ])
    assert.throws(() => webPolicy({ ...env, BRIDGE_CSP_CONNECT_ORIGINS: JSON.stringify([origin]) }, false));
  assert.throws(() => webPolicy({ ...env, VITE_FAUCET_ORIGIN: env.VITE_FUNDS_ORIGIN }, false));
  assert.throws(() => webPolicy({ ...env, VITE_FAUCET_ORIGIN: 'https://funds.example.test:8443' }, false));
  assert.throws(() =>
    secureHtml(html.replace('/assets/main.js', 'https://evil.test/main.js'), assets(), webPolicy(env, false)),
  );
  assert.throws(() =>
    secureHtml(html.replace('</body>', '<script>alert(1)</script></body>'), assets(), webPolicy(env, false)),
  );
  for (const [name, content] of [
    ['assets/lazy.js', 'alert(1)'],
    ['assets/worker.js', 'postMessage(1)'],
    ['assets/crypto.wasm', 'wasm'],
    ['nested.html', '<script>alert(1)</script>'],
  ]) {
    const resources = assets();
    resources.set(name, content);
    assert.throws(() => secureHtml(html, resources, webPolicy(env, false)));
  }
  for (const code of [
    'new Worker("worker.js")',
    'new SharedWorker("worker.js")',
    'navigator.serviceWorker.register("worker.js")',
  ]) {
    const resources = assets();
    resources.set('assets/main.js', code);
    assert.throws(() => secureHtml(html, resources, webPolicy(env, false)));
  }
  const externalCss = assets();
  externalCss.set('assets/main.css', '@import "https://evil.test/style.css";');
  assert.throws(() => secureHtml(html, externalCss, webPolicy(env, false)));
});

await test('HTML7fe1b032 Turnstile authority stays on the separately approved faucet', () => {
  const faucet = {
    ...env,
    VITE_ISOLATED_FAUCET: 'false',
    VITE_TURNSTILE_SITEKEY: 'offline-test-key',
    VITE_TURNSTILE_HOSTNAME: 'faucet.example.test',
  };
  assert.throws(() => webPolicy({ ...faucet, VITE_TURNSTILE_HOSTNAME: 'funds.example.test' }, true));
  assert.throws(() => webPolicy({ ...faucet, VITE_TURNSTILE_SITEKEY: '' }, true));
  assert(
    secureHtml(html, assets(), webPolicy(faucet, true)).csp.includes('frame-src https://challenges.cloudflare.com'),
  );
  assert(!secureHtml(html, assets(), webPolicy(faucet, false)).csp.includes('challenges.cloudflare.com'));
  assert(!secureHtml(html, assets(), webPolicy(env, true)).csp.includes('challenges.cloudflare.com'));
});
