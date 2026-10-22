import { viteConfigs } from '@gear-js/frontend-configs';
import { defineConfig, loadEnv, mergeConfig } from 'vite';

export default defineConfig(({ mode }) => {
  const { BRIDGE_INDEXER_PROXY } = loadEnv(mode, process.cwd(), 'BRIDGE_');
  let proxy;
  if (BRIDGE_INDEXER_PROXY) {
    const target = new URL(BRIDGE_INDEXER_PROXY);
    if (!['http:', 'https:'].includes(target.protocol) || target.username || target.password)
      throw new Error('BRIDGE_INDEXER_PROXY must be an HTTP(S) URL without credentials');
    proxy = { '/graphql': { target: target.href, ws: true } };
  }
  return mergeConfig(viteConfigs.app, {
    css: { preprocessorOptions: { scss: { additionalData: '@use "@/utils/breakpoints" as *;' } } },
    server: { proxy },
    preview: { proxy },
  });
});
