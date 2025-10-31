import {
  ApiProvider as GearApiProvider,
  AccountProvider as GearAccountProvider,
  AlertProvider as GearAlertProvider,
  ProviderProps,
} from '@gear-js/react-hooks';
import { Alert, alertStyles } from '@gear-js/vara-ui';
import { AppKitNetwork } from '@reown/appkit/networks';
import * as allNetworks from '@reown/appkit/networks';
import { createAppKit } from '@reown/appkit/react';
import { WagmiAdapter } from '@reown/appkit-adapter-wagmi';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ComponentType } from 'react';
import { http, WagmiProvider } from 'wagmi';

import { NETWORK_PRESET, VARA_NODE_ADDRESS } from './consts';
import { TokensProvider } from './context';

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

const getNetwork = (id: number) => {
  const result = Object.values(allNetworks)
    .filter((network) => 'id' in network)
    .find((network) => network.id === id);

  if (!result) throw new Error(`Chain with id ${id} not found`);

  return result;
};

const networks: [AppKitNetwork, AppKitNetwork] = [
  getNetwork(NETWORK_PRESET.MAINNET.ETH_CHAIN_ID),
  getNetwork(NETWORK_PRESET.TESTNET.ETH_CHAIN_ID),
];

const metadata = {
  name: 'Vara Network Bridge',
  description: 'Bridge Vara and Ethereum with ease',
  url: 'https://bridge.vara.network',
  icons: [
    'https://raw.githubusercontent.com/gear-tech/gear-bridges/refs/heads/main/js/frontend/public/favicons/web-app-manifest-512x512.png',
  ],
};

const adapter = new WagmiAdapter({
  networks,
  projectId,

  transports: {
    [NETWORK_PRESET.MAINNET.ETH_CHAIN_ID]: http(NETWORK_PRESET.MAINNET.ETH_NODE_ADDRESS),
    [NETWORK_PRESET.TESTNET.ETH_CHAIN_ID]: http(NETWORK_PRESET.TESTNET.ETH_NODE_ADDRESS),
  },
});

const METAMASK_WALLET_ID = 'c57ca95b47569778a828d19178114f4db188b89b763c899ba0be274e97267d96';
const COINBASE_WALLET_ID = 'fd20dc426fb37566d803205b19bbc1d4096b248ac04548e3cfb6b3a38bd033aa';
const TRUST_WALLET_ID = '4622a2b2d6af1c9844944291e5e7351a6aa24cd7b23099efac1b2fd875da31a0';

createAppKit({
  adapters: [adapter],
  networks,
  metadata,
  projectId,
  features: { analytics: false, email: false, socials: false },
  enableWalletGuide: false,
  allWallets: 'HIDE',
  excludeWalletIds: [TRUST_WALLET_ID],
  includeWalletIds: [METAMASK_WALLET_ID, COINBASE_WALLET_ID],
  themeMode: 'dark',
  themeVariables: {
    '--w3m-font-family': 'Geist Variable',
    '--w3m-border-radius-master': '1px',
  },
});

function EthProvider({ children }: ProviderProps) {
  return <WagmiProvider config={adapter.wagmiConfig}>{children}</WagmiProvider>;
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

const providers = [ApiProvider, AccountProvider, AlertProvider, EthProvider, QueryProvider, TokensProvider];

const WithProviders = (Component: ComponentType) => () =>
  providers.reduceRight((children, Provider) => <Provider>{children}</Provider>, <Component />);

// eslint-disable-next-line react-refresh/only-export-components
export { WithProviders, getNetwork };
