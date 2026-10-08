import { defineConfig } from 'vitest/config';

export default defineConfig(({ mode }) => ({
  test: {
    environment: 'node',
    include: ['test/**/*.test.ts'],
    // Explicit codec/evidence unit mode does not qualify the full owned live-fixture gates.
    ...(mode === 'unit' ? { testNamePattern: '^SDK ' } : { globalSetup: ['./test/setup/setup.ts'] }),
  },
}));
