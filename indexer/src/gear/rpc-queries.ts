import { RpcClient } from '@subsquid/rpc-client';
import { Runtime } from '@subsquid/substrate-runtime';
import { encodeName } from '@subsquid/substrate-runtime/lib/runtime/storage';

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
