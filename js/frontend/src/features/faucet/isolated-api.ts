import { z } from 'zod';

const ISOLATED_GENESIS = '0xa12f631344f61644239b70f9ab53e4842e540d8b0c1e7adb9570c06a62c3af3f';
const ISOLATED_CHAIN_ID = 560048;
const ISOLATED_USDC = '0xd4d65b1100b522cbde9664b9ba78c8add04e5b53';

const configSchema = z.object({
  genesis: z.literal(ISOLATED_GENESIS),
  chainId: z.literal(ISOLATED_CHAIN_ID),
  assets: z.object({
    TVARA: z.object({ amount: z.literal('100000000000000'), decimals: z.literal(12) }),
    USDC: z.object({
      amount: z.literal('1000000000'),
      decimals: z.literal(6),
      contract: z.literal(ISOLATED_USDC),
    }),
  }),
  cooldownMs: z.literal(86400000),
});

const claimSchema = z
  .object({
    status: z.enum(['idle', 'pending', 'finalized', 'failed']),
    txHash: z
      .string()
      .regex(/^0x[0-9a-fA-F]{64}$/)
      .optional(),
    nextClaimAt: z.number().int().nonnegative().optional(),
    error: z.string().optional(),
  })
  .refine((claim) => !['pending', 'finalized'].includes(claim.status) || Boolean(claim.txHash), {
    message: 'Claim transaction hash is missing',
  });

const pendingSchema = claimSchema.refine((claim) => claim.status === 'pending' && claim.nextClaimAt !== undefined, {
  message: 'Expected a pending claim with a cooldown',
});

type IsolatedAsset = 'TVARA' | 'USDC';
type IsolatedConfig = z.infer<typeof configSchema>;
type IsolatedClaim = z.infer<typeof claimSchema>;

async function faucetFetch(path: string, init?: RequestInit): Promise<unknown> {
  const response = await fetch(`/faucet/${path}`, {
    ...init,
    credentials: 'same-origin',
    mode: 'same-origin',
    redirect: 'error',
    cache: 'no-store',
  });
  const body: unknown = await response.json();

  if (!response.ok) {
    const error = z.object({ error: z.string() }).safeParse(body);

    throw new Error(error.success ? error.data.error : 'Isolated faucet request failed');
  }

  return body;
}

async function getIsolatedConfig() {
  const parsed = configSchema.safeParse(await faucetFetch('config'));

  if (!parsed.success) throw new Error('Isolated faucet configuration does not match the approved test network');

  return parsed.data;
}

async function getIsolatedStatus(asset: IsolatedAsset, address: string) {
  const query = new URLSearchParams({ asset, address });

  return claimSchema.parse(await faucetFetch(`status?${query}`));
}

async function requestIsolatedBalance(asset: IsolatedAsset, address: string, config: IsolatedConfig) {
  // Validate again at the write boundary; never submit a mainnet or substitute-token request.
  const { genesis, chainId } = configSchema.parse(config);

  return pendingSchema.parse(
    await faucetFetch('request', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ asset, address, genesis, chainId }),
    }),
  );
}

export { ISOLATED_CHAIN_ID, ISOLATED_USDC, getIsolatedConfig, getIsolatedStatus, requestIsolatedBalance };
export type { IsolatedAsset, IsolatedConfig, IsolatedClaim };
