import { useAccount, useApi } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';
import { ComponentProps, useState } from 'react';
import { FormProvider } from 'react-hook-form';

import { Input } from '@/components';
import { TransactionModal } from '@/features/history/components/transaction-modal';
import { Network as TransferNetwork } from '@/features/history/types';
import { TokenPrice } from '@/features/token-price';
import { useEthAccount, useModal, useVaraSymbol } from '@/hooks';
import { isUndefined } from '@/utils';

import PlusSVG from '../../assets/plus.svg?react';
import { FIELD_NAME } from '../../consts';
import { useBridgeContext } from '../../context';
import { useSwapForm, useToken } from '../../hooks';
import { UseHandleSubmit, UseAccountBalance, UseFTBalance, UseFee, UseFTAllowance } from '../../types';
import { AmountInput } from '../amount-input';
import { Balance } from '../balance';
import { DetailsAccordion } from '../details-accordion';
import { SubmitProgressBar } from '../submit-progress-bar';
import { SubmitTooltip } from '../submit-tooltip';
import { SwapNetworkButton } from '../swap-network-button';
import { Token } from '../token';

import styles from './swap-form.module.scss';

type Props = {
  useAccountBalance: UseAccountBalance;
  useFTBalance: UseFTBalance;
  useFTAllowance: UseFTAllowance;
  useHandleSubmit: UseHandleSubmit;
  useFee: UseFee;
};

function SwapForm({ useHandleSubmit, useAccountBalance, useFTBalance, useFTAllowance, useFee }: Props) {
  const { network, pair, token } = useBridgeContext();
  const { index: networkIndex, isVara: isVaraNetwork } = network;
  const { index: pairIndex } = pair;
  const { isNative: isNativeToken } = token;

  const { api } = useApi();
  const { address, destinationAddress, destinationSymbol, symbol, decimals, ...bridge } = useToken(
    networkIndex,
    pairIndex,
  );

  const { fee, ...config } = useFee();
  const accountBalance = useAccountBalance();
  const ftBalance = useFTBalance(address);
  const allowance = useFTAllowance(address);

  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const isNetworkAccountConnected = (network.isVara && Boolean(account)) || (!network.isVara && ethAccount.isConnected);

  const { open: openEthWalletModal } = useAppKit();
  const [isSubstrateWalletModalOpen, openSubstrateWalletModal, closeSubstrateWalletModal] = useModal();
  const [transactionModal, setTransactionModal] = useState<ComponentProps<typeof TransactionModal> | undefined>();

  const varaSymbol = useVaraSymbol();

  const openTransacionModal = (amount: string, receiver: string) => {
    if (!address || !destinationAddress) throw new Error('Address is not defined');

    const source = address;
    const destination = destinationAddress;
    const sourceNetwork = isVaraNetwork ? TransferNetwork.Gear : TransferNetwork.Ethereum;
    const destNetwork = isVaraNetwork ? TransferNetwork.Ethereum : TransferNetwork.Gear;
    const sender = isVaraNetwork ? account!.decodedAddress : ethAccount.address!;
    const close = () => setTransactionModal(undefined);

    setTransactionModal({ amount, source, destination, sourceNetwork, destNetwork, sender, receiver, close });
  };

  const [submit, approve, payFee, mint] = useHandleSubmit(
    fee.value,
    allowance.data,
    ftBalance.data,
    accountBalance.data,
    openTransacionModal,
  );

  const { form, amount, handleSubmit, setMaxBalance } = useSwapForm(
    isVaraNetwork,
    accountBalance,
    ftBalance,
    decimals,
    submit.mutateAsync,
  );

  const renderFromBalance = () => {
    const balance = isNativeToken ? accountBalance : ftBalance;

    return (
      <Balance
        value={balance.data}
        decimals={decimals}
        symbol={symbol}
        isLoading={balance.isLoading || bridge.isLoading}
        onMaxButtonClick={setMaxBalance}
      />
    );
  };

  const isEnoughBalance = () => {
    if (!api || isUndefined(fee.value) || !accountBalance.data) return false;

    const requiredBalance = isVaraNetwork ? fee.value + api.existentialDeposit.toBigInt() : fee.value;

    return accountBalance.data > requiredBalance;
  };

  const getButtonText = () => {
    if (!isEnoughBalance()) return `Not Enough ${isVaraNetwork ? varaSymbol : 'ETH'}`;

    if (approve.isPending) return 'Approving...';
    if (submit.isPending) return 'Transferring...';

    return 'Transfer';
  };

  const handleConnectWalletButtonClick = () => {
    const openWalletModal = isVaraNetwork ? openSubstrateWalletModal : openEthWalletModal;

    void openWalletModal();
  };

  const renderTokenPrice = () => {
    // to map through token ids without storing eth addresses
    const varaAddress = network.isVara ? address : destinationAddress;

    return <TokenPrice address={varaAddress} amount={amount} />;
  };

  const renderProgressBar = () => <SubmitProgressBar mint={mint} approve={approve} submit={submit} payFee={payFee} />;

  return (
    <>
      <FormProvider {...form}>
        <form onSubmit={handleSubmit} className={styles.form}>
          <div>
            <div className={styles.card}>
              <header className={styles.header}>
                <h3 className={styles.heading}>From</h3>
                <AmountInput.Error />
              </header>

              <div className={styles.row}>
                <Token
                  type="select"
                  address={address}
                  symbol={symbol}
                  network={isVaraNetwork ? 'Vara Testnet' : 'Ethereum Holesky'}
                  networkIndex={networkIndex}
                />

                <AmountInput />
              </div>

              <div className={styles.balanceFooter}>
                {renderFromBalance()}
                {renderTokenPrice()}
              </div>

              <SwapNetworkButton />
            </div>

            <div className={styles.card}>
              <h3 className={styles.heading}>To</h3>

              <div className={styles.row}>
                <Token
                  type="text"
                  address={destinationAddress}
                  symbol={destinationSymbol}
                  network={isVaraNetwork ? 'Ethereum Holesky' : 'Vara Testnet'}
                  networkIndex={Number(!networkIndex)}
                />

                <AmountInput.Value />
              </div>

              <div className={styles.priceFooter}>{renderTokenPrice()}</div>

              <Input
                icon={PlusSVG}
                name={FIELD_NAME.ADDRESS}
                label="Bridge to"
                className={styles.input}
                spellCheck={false}
                block
              />
            </div>
          </div>

          <DetailsAccordion isVaraNetwork={isVaraNetwork} />

          {isNetworkAccountConnected ? (
            <SubmitTooltip allowance={allowance.data} decimals={decimals} symbol={symbol} amount={amount}>
              <Button
                type="submit"
                text={getButtonText()}
                disabled={!isEnoughBalance()}
                isLoading={
                  mint?.isPending ||
                  payFee?.isPending ||
                  submit.isPending ||
                  accountBalance.isLoading ||
                  ftBalance.isLoading ||
                  config.isLoading ||
                  bridge.isLoading ||
                  allowance.isLoading
                }
                block
              />
            </SubmitTooltip>
          ) : (
            <Button type="button" text="Connect Wallet" onClick={handleConnectWalletButtonClick} block />
          )}
        </form>
      </FormProvider>

      {isSubstrateWalletModalOpen && <WalletModal close={closeSubstrateWalletModal} />}
      {transactionModal && <TransactionModal renderProgressBar={renderProgressBar} {...transactionModal} />}
    </>
  );
}

export { SwapForm };
