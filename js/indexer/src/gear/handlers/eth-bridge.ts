import { EthBridgeMessageQueuedContext } from '../types/index.js';
import { GearEthBridgeMessage } from '../../model/index.js';
import { gearNonce } from '../../common/index.js';

export function handleEthBridgeMessage(ctx: EthBridgeMessageQueuedContext) {
  const {
    message: { nonce },
    hash,
  } = ctx.event.args;

  ctx.state.addEthBridgeMessage(
    new GearEthBridgeMessage({
      id: hash,
      nonce: gearNonce(BigInt(nonce)),
      blockNumber: BigInt(ctx.blockHeader.height),
    }),
  );
}
