import {
  ApiProvider as GearApiProvider,
  AccountProvider as GearAccountProvider,
  AlertProvider as GearAlertProvider,
  ProviderProps,
} from '@gear-js/react-hooks';
import { Alert, alertStyles } from '@gear-js/vara-ui';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { createWeb3Modal } from '@web3modal/wagmi/react';
import { defaultWagmiConfig } from '@web3modal/wagmi/react/config';
import { ComponentType } from 'react';
import { WagmiProvider, http } from 'wagmi';
import * as allChains from 'wagmi/chains';

import { VARA_NODE_ADDRESS, ETH_CHAIN_ID, ETH_NODE_ADDRESS } from './consts';
import { BridgeProvider } from './contexts';

function ApiProvider({ children }: ProviderProps) {
  return <GearApiProvider initialArgs={{ endpoint: VARA_NODE_ADDRESS }}>{children}</GearApiProvider>;
}

function AccountProvider({ children }: ProviderProps) {
  return <GearAccountProvider appName="Vara Network Bridge">{children}</GearAccountProvider>;
}

function AlertProvider({ children }: ProviderProps) {
  return (
    <GearAlertProvider template={Alert} containerClassName={alertStyles.root}>
      {children}
    </GearAlertProvider>
  );
}

const projectId = import.meta.env.VITE_WALLET_CONNECT_PROJECT_ID as string;

const metadata = {
  name: 'Web3Modal',
  description: 'Web3Modal Example',
  url: 'https://vara.network/', // origin must match your domain & subdomain
  icons: ['https://avatars.githubusercontent.com/u/37784886'],
};

const getChain = (id: number) => {
  const result = Object.values(allChains).find((chain) => chain.id === Number(id));

  if (!result) throw new Error(`Chain with id ${id} not found`);

  return result;
};

const chains = [getChain(ETH_CHAIN_ID)] as const;

const config = defaultWagmiConfig({
  chains,
  projectId,
  metadata,
  transports: { [ETH_CHAIN_ID]: http(ETH_NODE_ADDRESS) },
  auth: { email: false },
});

createWeb3Modal({
  projectId,
  // @ts-expect-error -- revisit after wagmi and web3modal bumps
  wagmiConfig: config,
  enableAnalytics: false,
  allWallets: 'HIDE',
  themeMode: 'light',
  themeVariables: {
    '--w3m-font-family': 'Anuphan',
    '--w3m-border-radius-master': '1px',
  },
});

declare module 'wagmi' {
  interface Register {
    config: typeof config;
  }
}

function EthProvider({ children }: ProviderProps) {
  // @ts-expect-error -- revisit after wagmi and web3modal bumps
  return <WagmiProvider config={config}>{children}</WagmiProvider>;
}

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      gcTime: 0,
      staleTime: Infinity,
      refetchOnWindowFocus: false,
    },
  },
});

function QueryProvider({ children }: ProviderProps) {
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}

const providers = [ApiProvider, AccountProvider, AlertProvider, EthProvider, QueryProvider, BridgeProvider];

const WithProviders = (Component: ComponentType) => () =>
  providers.reduceRight((children, Provider) => <Provider>{children}</Provider>, <Component />);

export { WithProviders };
