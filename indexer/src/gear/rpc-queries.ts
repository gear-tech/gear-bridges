import { RpcClient } from '@subsquid/rpc-client';
import { Runtime } from '@subsquid/substrate-runtime';
import { encodeName } from '@subsquid/substrate-runtime/lib/runtime/storage';
import { ZERO_ADDRESS } from 'sails-js';
import { Decoder } from './codec';
import { ethers } from 'ethers';
import { config } from './config';

export async function getProgramInheritor(
  rpc: RpcClient,
  runtime: Runtime,
  programId: string,
  blockhash: string,
): Promise<string> {
  const module = 'GearProgram';
  const method = 'ProgramStorage';

  const param = encodeName(module, method) + programId.slice(2);

  const response = await rpc.call('state_getStorage', [param, blockhash]);

  const program = runtime.decodeStorageValue(`${module}.${method}`, response);

  if (program.__kind == 'Exited') {
    return program.value;
  } else {
    return programId;
  }
}

let vftDecoder: Decoder;

export async function getVaraTokenSymbol(rpc: RpcClient, programId: string, blockhash: string): Promise<string> {
  const method = 'gear_calculateReplyForHandle';
  const origin = ZERO_ADDRESS;
  const gasLimit = 1e10;
  const value = 0;

  const service = 'VftMetadata';
  const fn = 'Symbol';

  const payload = vftDecoder.encodeQueryInput(service, fn, []);

  const response = await rpc.call(method, [origin, programId, payload, gasLimit, value, blockhash]);

  if (response.code.Success) {
    const symbol = vftDecoder.decodeQueryOutput<string>(service, fn, response.payload);
    return symbol;
  } else {
    throw new Error(`Failed to get token symbol. ${response.code}`);
  }
}

export async function initDecoders() {
  vftDecoder = await Decoder.create('./assets/vft.idl');
}

export async function getEthTokenSymbol(contractAddress: string): Promise<string> {
  const erc20ABI = ['function symbol() view returns (string)'];

  const provider = new ethers.JsonRpcProvider(config.ethRpcUrl);

  try {
    const tokenContract = new ethers.Contract(contractAddress, erc20ABI, provider);

    const symbol = await tokenContract.symbol();
    return symbol;
  } catch (error) {
    throw new Error(`Failed to get ERC20 token symbol: ${error instanceof Error ? error.message : String(error)}`);
  }
}
