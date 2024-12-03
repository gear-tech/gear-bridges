import { getTypedEntries } from '@gear-js/react-hooks';

import EnkryptSVG from './assets/enkrypt.svg?react';
import PolkadotSVG from './assets/polkadot.svg?react';
import SubWalletSVG from './assets/subwallet.svg?react';
import TalismanSVG from './assets/talisman.svg?react';

const WALLET = {
  'polkadot-js': { name: 'Polkadot JS', SVG: PolkadotSVG },
  'subwallet-js': { name: 'SubWallet', SVG: SubWalletSVG },
  talisman: { name: 'Talisman', SVG: TalismanSVG },
  enkrypt: { name: 'Enkrypt', SVG: EnkryptSVG },
};

const WALLETS = getTypedEntries(WALLET);

export { WALLET, WALLETS };
