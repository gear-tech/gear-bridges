import { RpcClient } from '@subsquid/rpc-client';
import { Runtime } from '@subsquid/substrate-runtime';
import { encodeName } from '@subsquid/substrate-runtime/lib/runtime/storage';
import { ZERO_ADDRESS } from 'sails-js';
import { Decoder } from './codec';
import { ethers } from 'ethers';
import { config } from './config';

let vftDecoder: Decoder;

export async function initDecoders() {
  vftDecoder = await Decoder.create('./assets/vft.idl');
}

// Helper function for querying VFT metadata
async function queryVFTMetadata<T>(rpc: RpcClient, programId: string, blockhash: string, fn: string): Promise<T> {
  const method = 'gear_calculateReplyForHandle';
  const origin = ZERO_ADDRESS;
  const gasLimit = 1e10;
  const value = 0;

  const service = 'VftMetadata';

  const payload = vftDecoder.encodeQueryInput(service, fn, []);

  const response = await rpc.call(method, [origin, programId, payload, gasLimit, value, blockhash]);

  if (response.code.Success) {
    const result = vftDecoder.decodeQueryOutput<T>(service, fn, response.payload);
    return result;
  } else {
    throw new Error(`Failed to get token ${fn}. ${response.code}`);
  }
}

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

export async function getVaraTokenDecimals(rpc: RpcClient, programId: string, blockhash: string): Promise<number> {
  return queryVFTMetadata(rpc, programId, blockhash, 'Decimals');
}

export async function getVaraTokenName(rpc: RpcClient, programId: string, blockhash: string): Promise<string> {
  return queryVFTMetadata<string>(rpc, programId, blockhash, 'Name');
}

export async function getVaraTokenSymbol(rpc: RpcClient, programId: string, blockhash: string): Promise<string> {
  return queryVFTMetadata<string>(rpc, programId, blockhash, 'Symbol');
}

// Helper function for querying Ethereum ERC20 token properties
async function queryEthTokenProperty<T>(contractAddress: string, method: string, abi: string[]): Promise<T> {
  const provider = new ethers.JsonRpcProvider(config.ethRpcUrl);

  try {
    const tokenContract = new ethers.Contract(contractAddress, abi, provider);
    const result: T = await tokenContract[method]();
    return result;
  } catch (error) {
    throw new Error(`Failed to get ERC20 token ${method}: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function getEthTokenSymbol(contractAddress: string): Promise<string> {
  const erc20ABI = ['function symbol() view returns (string)'];
  return queryEthTokenProperty<string>(contractAddress, 'symbol', erc20ABI);
}

export async function getEthTokenDecimals(contractAddress: string): Promise<number> {
  const erc20ABI = ['function decimals() view returns (uint8)'];
  return queryEthTokenProperty<number>(contractAddress, 'decimals', erc20ABI);
}

export async function getEthTokenName(contractAddress: string): Promise<string> {
  const erc20ABI = ['function name() view returns (string)'];
  return queryEthTokenProperty<string>(contractAddress, 'name', erc20ABI);
}
