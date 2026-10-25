import type { GearApi, HexString } from '@gear-js/api';
import {
  immutableInboundProfile,
  nativeWrapperIdlSha256,
  type InboundProofProfile,
  type NativeRedemption,
} from '@gear-js/bridge';
import { TypeRegistry } from '@polkadot/types';
import { QueryBuilder } from 'sails-js';

/** Only Redemption is shared by the two approved IDLs; never probe a hold/count route to infer capability. */
export function createNativeRedemptionReader(api: GearApi, supplied: InboundProofProfile) {
  const profile = immutableInboundProfile(supplied);
  const wrapper = profile.nativeWrapper;
  if (!wrapper || wrapper.idlSha256 !== nativeWrapperIdlSha256(wrapper.mode)) {
    throw new Error('HOLD: approved native wrapper capability and deployed IDL are required');
  }
  // Both exact IDL artifacts declare this identical read-only tuple. No v2-only service is exposed.
  const registry = new TypeRegistry();
  registry.register({
    Redemption: {
      from: '[u8;32]',
      to: '[u8;32]',
      amount: 'U256',
      child: '[u8;32]',
      status: 'PayoutStatus',
      returned_value: 'u128',
    },
    PayoutStatus: { _enum: ['Queued', 'Delivered', 'Returned', 'Ambiguous', 'Refunded'] },
  });
  return (operationId: HexString, pin: HexString): Promise<NativeRedemption | null> =>
    new QueryBuilder<NativeRedemption | null>(
      api,
      registry,
      wrapper.programId,
      'NativeEscrow',
      'Redemption',
      operationId,
      'H256',
      'Option<Redemption>',
    )
      .atBlock(pin)
      .call();
}
