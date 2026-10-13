import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const commonjs = require('@gear-js/bridge');
const esm = await import('@gear-js/bridge');

for (const [entry, sdk] of [['require', commonjs], ['import', esm]]) {
  for (const name of ['relayEthToVara', 'relayVaraToEth']) {
    assert.equal(typeof sdk[name], 'function', `${entry}('@gear-js/bridge').${name}`);
  }
  assert.deepEqual(sdk.decodeEthBridgeMessageResponse(new Uint8Array(77)), {
    blockNumber: 0n, hash: `0x${'00'.repeat(32)}`, nonce: 0n, queueId: 0n,
  });
}
console.log('Package entries passed: require/import expose both relay functions and decode the queued-message response');
