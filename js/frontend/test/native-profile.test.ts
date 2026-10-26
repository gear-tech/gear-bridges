import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';

import type { GearApi, HexString } from '@gear-js/api';
import {
  immutableInboundProfile,
  nativeWrapperIdlSha256,
  type InboundProofProfile,
  type NativeWrapperMode,
} from '@gear-js/bridge';
import { nativeAssetKind, validateTokenPairInventory } from '@gear-js/bridge';
import { Sails } from 'sails-js';
import { SailsIdlParser } from 'sails-js-parser';

import { assertCurrentToken } from '../src/context/token-selection.ts';
import type { Token } from '../src/context/tokens';
import { createNativeRedemptionReader } from '../src/features/swap/native-redemption.ts';

const actor: HexString = `0x${'ab'.repeat(32)}`;
const token: HexString = `0x${'cd'.repeat(20)}`;
const upperActor: HexString = `0x${actor.slice(2).toUpperCase()}`;
const idl = (name: string) => readFileSync(new URL(`../../../api/gear/${name}.idl`, import.meta.url), 'utf8');
const digest = (text: string) => createHash('sha256').update(text).digest('hex');
const parser = await SailsIdlParser.new();
for (const mode of ['escrow-v1', 'held-escrow-v2'] as const) {
  const wrapperIdl = idl(mode === 'escrow-v1' ? 'vft_vara_v1' : 'vft_vara');
  assert.equal(digest(wrapperIdl), nativeWrapperIdlSha256(mode), 'Pin the real selected IDL, not a modern alias');
  const wrapper = new Sails(parser).parseIdl(wrapperIdl);
  if (mode === 'escrow-v1') {
    assert.equal(wrapper.services.NativeEscrow.queries.BlockingRedemptions, undefined);
    assert.equal(wrapper.services.NativeEscrow.queries.HeldRedemption, undefined);
    assert.equal(wrapper.services.NativeEscrow.functions.HoldRedemption, undefined);
  }
  const profile: InboundProofProfile = {
    ethereumChainId: 1n,
    ethereumGenesisHash: upperActor,
    beaconGenesisValidatorsRoot: upperActor,
    beaconGenesisTime: 0n,
    sourceGenesisHash: upperActor,
    sourceBlockHash: upperActor,
    sourceRuntimeCodeHash: upperActor,
    historicalProxyId: upperActor,
    historicalProxyCodeId: upperActor,
    historicalProxyIdlSha256: digest(idl('historical_proxy')).toUpperCase(),
    endpoint: {
      programId: upperActor,
      codeId: upperActor,
      idlSha256: digest(idl('eth_events_electra')),
      framing: 'electra',
    },
    checkpoint: {
      programId: upperActor,
      codeId: upperActor,
      idlSha256: digest(idl('checkpoint_light_client')),
      network: 'Mainnet',
    },
    consumer: {
      programId: upperActor,
      codeId: upperActor,
      idlSha256: digest(idl('vft_manager')),
      service: 'VftManager',
      method: 'SubmitReceipt',
    },
    nativeWrapper: {
      programId: upperActor,
      codeId: upperActor,
      ethereumToken: token,
      mode,
      idlSha256: digest(wrapperIdl).toUpperCase(),
    },
    forks: [{ name: 'electra', epoch: 0n, version: '0xABCDEFAB' }],
  };
  // Reconstructed from supplied HTML, audited 7fe1b032: VB-38/39.
  const approved = immutableInboundProfile(profile);
  assert.equal(nativeAssetKind(approved, actor, token, 'Gear'), 'vara');
  assert.equal(nativeAssetKind(approved, `0x${'99'.repeat(32)}`, token, 'Gear'), undefined);
  assert.equal(nativeAssetKind(approved, actor, token, 'Ethereum'), undefined);
  const pair = { varaToken: actor, ethToken: token, tokenSupply: 'Gear', isActive: true, symbol: 'ETH VARA USDC' };
  assert.doesNotThrow(() => validateTokenPairInventory([pair], [[actor, token, 'Gear']]));
  assert.throws(
    () => validateTokenPairInventory([{ ...pair, varaToken: `0x${'99'.repeat(32)}` }], [[actor, token, 'Gear']]),
    /HOLD/,
  );
  assert.throws(() => validateTokenPairInventory([pair, pair], [[actor, token, 'Gear']]), /HOLD/);
  assert.equal(approved.nativeWrapper!.programId, actor);
  assert.equal(approved.nativeWrapper!.idlSha256, digest(wrapperIdl));
  assert.equal(Object.isFrozen(approved.nativeWrapper), true);
  const operation: HexString = `0x${'ef'.repeat(32)}`;
  const child: HexString = `0x${'de'.repeat(32)}`;
  let calls = 0;
  const api = {
    blockGasLimit: { toBigInt: () => 100000n },
    specVersion: 1,
    message: {
      calculateReply: (args: { destination: HexString; at: HexString; payload: Uint8Array; value: bigint }) => {
        calls++;
        assert.equal(args.destination, actor);
        assert.equal(args.at, actor);
        assert.equal(args.value, 0n);
        assert.deepEqual(wrapper.registry.createType('(String,String,H256)', args.payload).toJSON(), [
          'NativeEscrow',
          'Redemption',
          operation,
        ]);
        const bytes = wrapper.registry
          .createType('(String,String,Option<Redemption>)', [
            'NativeEscrow',
            'Redemption',
            { from: actor, to: actor, amount: 10n, child, status: 'Refunded', returned_value: 10n },
          ])
          .toU8a();
        return Promise.resolve({ code: { isSuccess: true }, payload: Object.assign(bytes, { toU8a: () => bytes }) });
      },
    },
  } as unknown as GearApi;
  const read = createNativeRedemptionReader(api, profile);
  const redemption = await read(operation, actor);
  assert.equal(redemption!.status, 'Refunded');
  assert.equal(BigInt(redemption!.returned_value), 10n);
  assert.equal(redemption!.child, child);
  assert.equal(calls, 1, 'Only the approved Redemption route may be queried');
  for (const changed of [
    { ...profile, nativeWrapper: undefined },
    {
      ...profile,
      nativeWrapper: {
        ...profile.nativeWrapper!,
        mode: (mode === 'escrow-v1' ? 'held-escrow-v2' : 'escrow-v1') as NativeWrapperMode,
      },
    },
    { ...profile, nativeWrapper: { ...profile.nativeWrapper!, idlSha256: '0'.repeat(64) } },
    { ...profile, nativeWrapper: { ...profile.nativeWrapper!, ethereumToken: '0x00' as HexString } },
  ])
    assert.throws(() => createNativeRedemptionReader(api, changed), /HOLD/);
  assert.equal(calls, 1, 'Rejected capability bindings must not query or sign');
}
// Reconstructed from supplied HTML 7fe1b032: late indexer/profile replies cannot enable a wallet operation.
const selected: Token = {
  address: actor,
  destinationAddress: token,
  name: 'spoof ETH',
  symbol: 'USDC',
  displaySymbol: 'USDC',
  decimals: 12,
  isNative: true,
  supply: 'Gear',
  network: 'vara',
  isActive: true,
};
const selection = {
  api: {},
  client: {},
  NETWORK_PRESET: {},
  pairs: [selected],
  selectedToken: selected,
  getActiveToken: () => selected,
};
assert.doesNotThrow(() => assertCurrentToken(selection, selection, selected));
for (const changed of [
  { ...selection, selectedToken: undefined },
  { ...selection, selectedToken: { ...selected, address: `0x${'99'.repeat(32)}` as const } },
  { ...selection, NETWORK_PRESET: {} },
  { ...selection, api: {} },
  { ...selection, client: {} },
  { ...selection, pairs: undefined },
  { ...selection, getActiveToken: () => ({ ...selected, destinationAddress: `0x${'99'.repeat(20)}` as const }) },
  { ...selection, getActiveToken: () => ({ ...selected, supply: 'Ethereum' as const }) },
  { ...selection, getActiveToken: () => ({ ...selected, isActive: false }) },
]) {
  let signed = false;
  await assert.rejects(async () => {
    await Promise.resolve();
    assertCurrentToken(selection, changed, selected);
    signed = true;
  }, /HOLD/);
  assert.equal(signed, false);
}
console.log(
  'Native profile assertions passed: exact v1/v2 IDLs, read-only shared codec, mode binding and case-normalized approvals',
);
