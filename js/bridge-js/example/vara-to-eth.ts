import { runApp } from './app.js';

runApp('vara-to-eth').catch((error: unknown) => {
  console.error(error instanceof Error && error.message.startsWith('HOLD:')
    ? error.message : 'HOLD: application operation did not reach verified finality; retain and reconcile its original intent.');
  process.exitCode = 1;
});
