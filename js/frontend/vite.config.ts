import { readFileSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';

import { viteConfigs } from '@gear-js/frontend-configs';
import { defineConfig, loadEnv, mergeConfig, type Plugin } from 'vite';
import { z } from 'zod';

import { secureHtml, webPolicy } from './web-policy';

const serveConfig = z.object({
  headers: z.tuple([
    z.object({
      source: z.literal('**'),
      headers: z.array(z.object({ key: z.string(), value: z.string() })),
    }),
  ]),
});

export default defineConfig(({ mode, command }) => {
  const env = loadEnv(mode, process.cwd(), '');
  const faucet = mode === 'faucet';
  const policy = command === 'build' ? webPolicy(env, faucet) : undefined;
  let proxy;
  if (!faucet && env.BRIDGE_INDEXER_PROXY) {
    const target = new URL(env.BRIDGE_INDEXER_PROXY);
    if (!['http:', 'https:'].includes(target.protocol) || target.username || target.password)
      throw new Error('BRIDGE_INDEXER_PROXY must be an HTTP(S) URL without credentials');
    proxy = { '/graphql': { target: target.href, ws: true } };
  }
  const security: Plugin = {
    name: 'bridge-entry-integrity',
    enforce: 'post',
    apply: 'build',
    generateBundle: {
      order: 'post',
      handler(_options, bundle) {
        if (!policy) throw new Error('Build security policy missing');
        const entries = Object.values(bundle).filter((item) => item.type === 'chunk');
        if (
          entries.length !== 1 ||
          entries[0].type !== 'chunk' ||
          !entries[0].isEntry ||
          entries[0].imports.length ||
          entries[0].dynamicImports.length
        )
          throw new Error('Only one closed JavaScript entry bundle is allowed');
        if (
          !faucet &&
          Object.keys(entries[0].modules).some((name) =>
            /react-turnstile|react-gtm-module|\/features\/faucet\//.test(name),
          )
        )
          throw new Error('Faucet/third-party script integration leaked into the funds bundle');
        const htmlName = faucet ? 'faucet.html' : 'index.html';
        const html = bundle[htmlName];
        if (!html || html.type !== 'asset') throw new Error('Expected HTML entry missing');
        const assets = new Map<string, string | Uint8Array>();
        for (const [name, asset] of Object.entries(bundle)) {
          if (name !== htmlName) assets.set(name, asset.type === 'chunk' ? asset.code : asset.source);
        }
        // Vite copies public assets without including them in Rollup's bundle.
        const publicDir = resolve('public');
        for (const entry of readdirSync(publicDir, { recursive: true, withFileTypes: true })) {
          if (entry.isDirectory()) continue;
          if (!entry.isFile()) throw new Error('Only regular public assets are permitted');
          const file = resolve(entry.parentPath, entry.name);
          const name = file.slice(publicDir.length + 1);
          if (!/\.(?:png|jpe?g|ico|svg|webp|avif|woff2?|webmanifest)$/i.test(name))
            throw new Error(`Unqualified public asset: ${name}`);
          assets.set(name, readFileSync(file));
        }
        const secured = secureHtml(String(html.source), assets, policy);
        html.source = secured.html;
        if (faucet) {
          delete bundle[htmlName];
          html.fileName = 'index.html';
          bundle['index.html'] = html;
        }
        const base = serveConfig.parse(JSON.parse(readFileSync(resolve('serve.json'), 'utf8')));
        base.headers[0].headers.push({ key: 'Content-Security-Policy', value: secured.csp });
        this.emitFile({ type: 'asset', fileName: 'serve.json', source: JSON.stringify(base, null, 2) });
        if (faucet) {
          // Install on the existing service host. This never creates another dispenser.
          let upstream = 'http://127.0.0.1:5485';
          if (env.VITE_ISOLATED_FAUCET !== 'true') {
            const url = new URL(env.BRIDGE_FAUCET_PUBLIC_UPSTREAM || '');
            if (
              url.protocol !== 'https:' ||
              url.username ||
              url.password ||
              url.search ||
              url.hash ||
              url.pathname !== '/'
            )
              throw new Error('Public payout service needs an approved HTTPS upstream origin');
            upstream = url.origin;
          }
          const host = new URL(policy.faucetOrigin).host;
          const hostname = new URL(policy.faucetOrigin).hostname;
          this.emitFile({
            type: 'asset',
            fileName: 'faucet.nginx.conf',
            source: `# Install in nginx's http context on the existing service host, behind HTTPS.
map "$request_method:$http_origin" $faucet_origin_allowed {
  default 0;
  "GET:" 1;
  "GET:${policy.faucetOrigin}" 1;
  "POST:${policy.faucetOrigin}" 1;
}
server {
  listen 127.0.0.1:5486;
  server_name ${hostname};
  if ($http_host != "${host}") { return 403; }
  root /srv/vara-faucet;
  auth_basic "Separate faucet ${hostname}";
  auth_basic_user_file /etc/nginx/faucet.htpasswd;
  add_header Content-Security-Policy "${secured.csp}" always;
  add_header X-Content-Type-Options nosniff always;
  add_header Referrer-Policy no-referrer always;
  add_header Cache-Control no-store always;
  location / { try_files $uri $uri/ /index.html; }
  location = /serve.json { deny all; }
  location = /faucet.nginx.conf { deny all; }
  location /faucet/ {
    if ($faucet_origin_allowed = 0) { return 403; }
    proxy_pass ${upstream}${env.VITE_ISOLATED_FAUCET === 'true' ? '' : '/'};
    proxy_set_header Host ${new URL(upstream).host};
    proxy_set_header Origin "${policy.faucetOrigin}";
    proxy_set_header Authorization "";
    proxy_set_header Cookie "";
    proxy_set_header X-Faucet-Authenticated $remote_user;
    proxy_ssl_server_name on;
    proxy_ssl_verify on;
    proxy_ssl_trusted_certificate /etc/ssl/certs/ca-certificates.crt;
    proxy_hide_header Set-Cookie;
    proxy_next_upstream off;
    proxy_hide_header Access-Control-Allow-Origin;
    proxy_hide_header Access-Control-Allow-Credentials;
    client_max_body_size 8k;
  }
}
`,
          });
        }
      },
    },
  };
  return mergeConfig(viteConfigs.app, {
    plugins: [
      security,
      {
        name: 'bridge-preview-headers',
        configurePreviewServer(server) {
          const config = serveConfig.parse(
            JSON.parse(readFileSync(resolve(faucet ? 'dist-faucet/serve.json' : 'dist/serve.json'), 'utf8')),
          );
          const headers: { key: string; value: string }[] = config.headers[0].headers;
          if (!headers.some((header) => header.key === 'Content-Security-Policy'))
            throw new Error('Preview requires generated production CSP');
          server.middlewares.use((_req, res, next) => {
            for (const header of headers) res.setHeader(header.key, header.value);
            next();
          });
        },
      },
    ],
    css: { preprocessorOptions: { scss: { additionalData: '@use "@/utils/breakpoints" as *;' } } },
    build: {
      outDir: faucet ? 'dist-faucet' : 'dist',
      cssCodeSplit: false,
      rollupOptions: {
        input: resolve(faucet ? 'faucet.html' : 'index.html'),
        output: { inlineDynamicImports: true, manualChunks: undefined },
      },
    },
    server: { proxy },
    preview: { proxy },
  });
});
