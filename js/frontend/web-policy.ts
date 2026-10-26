import { createHash } from 'node:crypto';

import { deploymentOrigin } from './src/utils/deployment-origin.ts';

export type WebPolicy = {
  fundsOrigin: string;
  faucetOrigin: string;
  connections: string[];
  turnstile: boolean;
  wasm: boolean;
};

export const integrity = (content: string | Uint8Array) =>
  `sha384-${createHash('sha384').update(content).digest('base64')}`;

export function webPolicy(env: Record<string, string>, faucet: boolean): WebPolicy {
  const fundsOrigin = deploymentOrigin(env.VITE_FUNDS_ORIGIN);
  const faucetOrigin = deploymentOrigin(env.VITE_FAUCET_ORIGIN);
  if (fundsOrigin === faucetOrigin || new URL(fundsOrigin).hostname === new URL(faucetOrigin).hostname)
    throw new Error('Funds and faucet require separate HTTPS hostnames and authentication scopes');
  const configured: unknown = JSON.parse(env.BRIDGE_CSP_CONNECT_ORIGINS || 'null');
  if (!Array.isArray(configured)) throw new Error('BRIDGE_CSP_CONNECT_ORIGINS must be an approved JSON array');
  const connections = [...new Set(configured.map((origin) => deploymentOrigin(origin, ['https:', 'wss:'])))];
  const turnstile = faucet && env.VITE_ISOLATED_FAUCET !== 'true';
  if (
    turnstile &&
    (!/^[A-Za-z0-9_-]+$/.test(env.VITE_TURNSTILE_SITEKEY || '') ||
      env.VITE_TURNSTILE_HOSTNAME !== new URL(faucetOrigin).hostname)
  )
    throw new Error('Public faucet requires operator-approved Turnstile site key and exact faucet hostname');
  if (!['', 'false', 'true'].includes(env.BRIDGE_CSP_WASM_EVAL || ''))
    throw new Error('BRIDGE_CSP_WASM_EVAL must be an explicitly qualified boolean');
  return { fundsOrigin, faucetOrigin, connections, turnstile, wasm: env.BRIDGE_CSP_WASM_EVAL === 'true' };
}

export function secureHtml(html: string, assets: Map<string, string | Uint8Array>, policy: WebPolicy) {
  const scripts = new Set<string>();
  let entryScripts = 0;
  const referenced = new Set<string>();
  const resource = (url: string) => {
    if (!/^\/[A-Za-z0-9_./-]+$/.test(url) || url.includes('..') || url.startsWith('//'))
      throw new Error(`Nonlocal executable/style entry: ${url}`);
    const name = url.slice(1);
    const content = assets.get(name);
    if (content === undefined) throw new Error(`Entry asset is not in the build: ${name}`);
    referenced.add(name);
    return integrity(content);
  };
  html = html.replace(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi, (_tag, attributes: string, body: string) => {
    const src = /\bsrc="([^"]+)"/.exec(attributes)?.[1];
    if (!src || !src.endsWith('.js') || body.trim() || /\bon\w+\s*=|\bintegrity\s*=/i.test(attributes))
      throw new Error('Inline or pre-authorized scripts are forbidden');
    const hash = resource(src);
    entryScripts += 1;
    scripts.add(`'${hash}'`);
    return `<script${attributes.replace(/\s+crossorigin(?:="[^"]*")?/g, '')} integrity="${hash}" crossorigin="anonymous"></script>`;
  });
  html = html.replace(/<link\b[^>]*>/gi, (tag) => {
    const rel = /\brel="([^"]+)"/.exec(tag)?.[1];
    if (!['stylesheet', 'modulepreload'].includes(rel || '')) return tag;
    const href = /\bhref="([^"]+)"/.exec(tag)?.[1];
    if (!href || !href.endsWith(rel === 'stylesheet' ? '.css' : '.js') || /\bintegrity\s*=/i.test(tag))
      throw new Error('Invalid style/module entry');
    const hash = resource(href);
    if (rel === 'modulepreload') scripts.add(`'${hash}'`);
    return tag
      .replace(/\s+crossorigin(?:="[^"]*")?/g, '')
      .replace(/\s*\/?>$/, ` integrity="${hash}" crossorigin="anonymous">`);
  });
  if (entryScripts !== 1 || scripts.size !== 1)
    throw new Error('Exactly one integrity-protected JavaScript entry is required');
  if (/<style\b|\son\w+\s*=/i.test(html)) throw new Error('Inline executable/style HTML is forbidden');
  for (const [name, content] of assets) {
    const text = typeof content === 'string' ? content : Buffer.from(content).toString('utf8');
    if (/\.(?:[cm]?js|css)$/i.test(name) && !referenced.has(name))
      throw new Error(`Unprotected executable/style asset: ${name}`);
    if (/\.(?:wasm|html?)$/i.test(name)) throw new Error(`Unbundled executable asset: ${name}`);
    if (/\.[cm]?js$/i.test(name)) {
      const code = text;
      // Literal imports are inlined by Rollup. Dormant framework computed imports
      // remain subject to the hash-only script CSP; they cannot authorize another module.
      if (
        /\bnew\s+(?:(?:window|globalThis|self)\s*\.\s*)?(?:SharedWorker|Worker)\s*\(|\b(?:serviceWorker\s*\.\s*register|importScripts)\s*\(/.test(
          code,
        )
      )
        throw new Error(`Dynamic executable/worker loading is not qualified: ${name}`);
    }
    if (/\.css$/i.test(name) && /@import|url\(\s*["']?\s*(?:https?:|\/\/)/i.test(text))
      throw new Error(`Styles must bundle/self-host every resource: ${name}`);
    if (
      /\.svg$/i.test(name) &&
      /<script\b|\son\w+\s*=|<foreignObject\b|(?:href|src)\s*=\s*["']\s*(?:https?:|\/\/|javascript:)/i.test(text)
    )
      throw new Error(`Active or external SVG resource: ${name}`);
  }
  const turnstile = policy.turnstile ? ' https://challenges.cloudflare.com' : '';
  const csp = [
    "default-src 'none'",
    `script-src ${[...scripts].join(' ')}${policy.wasm ? " 'wasm-unsafe-eval'" : ''}${turnstile}`,
    "script-src-attr 'none'",
    // Wallet components use CSS-in-JS and style attributes. This is not a script permission.
    `style-src 'self' 'unsafe-inline'`,
    "img-src 'self' data:",
    "font-src 'self' data:",
    `connect-src 'self' ${policy.connections.join(' ')}${turnstile}`.trim(),
    `frame-src ${policy.turnstile ? 'https://challenges.cloudflare.com' : "'none'"}`,
    "worker-src 'none'",
    "object-src 'none'",
    "base-uri 'none'",
    "frame-ancestors 'none'",
    "form-action 'none'",
    "manifest-src 'self'",
  ].join('; ');
  return { html, csp };
}
