import { runApp } from './app.js';

runApp('eth-to-vara').catch((error: unknown) => {
  console.error(error instanceof Error && error.message.startsWith('HOLD:')
    ? error.message : 'HOLD: application operation did not reach verified finality; retain and reconcile its original intent.');
  process.exitCode = 1;
});
