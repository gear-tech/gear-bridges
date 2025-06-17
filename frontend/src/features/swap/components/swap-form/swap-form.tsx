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
import { FIELD_NAME, NETWORK } from '../../consts';
import { useBridgeContext } from '../../context';
import { useSwapForm } from '../../hooks';
import { UseHandleSubmit, UseAccountBalance, UseFTBalance, UseFee, UseFTAllowance } from '../../types';
import { AmountInput } from '../amount-input';
import { Balance } from '../balance';
import { DetailsAccordion } from '../details-accordion';
import { SubmitProgressBar } from '../submit-progress-bar';
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
  const { network, token, destinationToken } = useBridgeContext();

  const { api } = useApi();

  const { fee, ...config } = useFee();
  const accountBalance = useAccountBalance();
  const ftBalance = useFTBalance(token?.address);
  const allowance = useFTAllowance(token?.address);

  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const isNetworkAccountConnected = (network.isVara && Boolean(account)) || (!network.isVara && ethAccount.isConnected);

  const { open: openEthWalletModal } = useAppKit();
  const [isSubstrateWalletModalOpen, openSubstrateWalletModal, closeSubstrateWalletModal] = useModal();
  const [transactionModal, setTransactionModal] = useState<ComponentProps<typeof TransactionModal> | undefined>();

  const varaSymbol = useVaraSymbol();

  const openTransacionModal = (amount: string, receiver: string) => {
    if (!token || !destinationToken) throw new Error('Address is not defined');

    const source = token.address;
    const destination = destinationToken.address;
    const sourceNetwork = network.isVara ? TransferNetwork.Vara : TransferNetwork.Ethereum;
    const destNetwork = network.isVara ? TransferNetwork.Ethereum : TransferNetwork.Vara;
    const sender = network.isVara ? account!.decodedAddress : ethAccount.address!;
    const close = () => setTransactionModal(undefined);

    setTransactionModal({ amount, source, destination, sourceNetwork, destNetwork, sender, receiver, close });
  };

  const { submit, approve, payFee, mint, permitUSDC } = useHandleSubmit(
    fee.value,
    allowance.data,
    accountBalance.data,
    openTransacionModal,
  );

  const { form, amount, handleSubmit, setMaxBalance } = useSwapForm(
    network.isVara,
    accountBalance,
    ftBalance,
    token?.decimals,
    submit.mutateAsync,
  );

  const renderFromBalance = () => {
    const balance = token?.isNative ? accountBalance : ftBalance;

    return (
      <Balance
        value={balance.data}
        decimals={token?.decimals}
        symbol={token?.symbol}
        isLoading={balance.isLoading}
        onMaxButtonClick={setMaxBalance}
      />
    );
  };

  const isEnoughBalance = () => {
    if (!api || isUndefined(fee.value) || !accountBalance.data) return false;

    const requiredBalance = network.isVara ? fee.value + api.existentialDeposit.toBigInt() : fee.value;

    return accountBalance.data > requiredBalance;
  };

  const getButtonText = () => {
    if (!isEnoughBalance()) return `Not Enough ${network.isVara ? varaSymbol : 'ETH'}`;

    if (approve?.isPending) return 'Approving...';
    if (submit.isPending) return 'Transferring...';

    return 'Transfer';
  };

  const handleConnectWalletButtonClick = () => {
    const openWalletModal = network.isVara ? openSubstrateWalletModal : openEthWalletModal;

    void openWalletModal();
  };

  const renderTokenPrice = () => <TokenPrice symbol={token?.symbol} amount={amount} />;

  const renderProgressBar = () => (
    <SubmitProgressBar mint={mint} approve={approve} submit={submit} payFee={payFee} permitUSDC={permitUSDC} />
  );

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
                  address={token?.address}
                  symbol={token?.symbol}
                  networkText={network.isVara ? 'Vara Testnet' : 'Ethereum Holesky'}
                  network={network.name}
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
                  address={destinationToken?.address}
                  symbol={destinationToken?.symbol}
                  networkText={network.isVara ? 'Ethereum Holesky' : 'Vara Testnet'}
                  network={network.name === NETWORK.VARA ? NETWORK.ETH : NETWORK.VARA}
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

          <DetailsAccordion isVaraNetwork={network.isVara} />

          {isNetworkAccountConnected ? (
            <Button
              type="submit"
              text={getButtonText()}
              disabled={!isEnoughBalance()}
              isLoading={
                submit.isPending ||
                accountBalance.isLoading ||
                ftBalance.isLoading ||
                config.isLoading ||
                allowance.isLoading
              }
              block
            />
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
