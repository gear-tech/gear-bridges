import { viteConfigs } from '@gear-js/frontend-configs';
import { mergeConfig } from 'vite';

export default mergeConfig(viteConfigs.app, {
  server: {
    host: '127.0.0.1',
  },
});
