import { useAccount, useApi } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';
import { ComponentProps, useState } from 'react';
import { FormProvider } from 'react-hook-form';

import { Input } from '@/components';
import { TokenPrice } from '@/features/token-price';
import { useEthAccount, useModal, useVaraSymbol } from '@/hooks';
import { isUndefined } from '@/utils';

import PlusSVG from '../../assets/plus.svg?react';
import { CLAIM_TYPE, FIELD_NAME, NETWORK, PRIORITY } from '../../consts';
import { useBridgeContext } from '../../context';
import { useSwapForm } from '../../hooks';
import { UseHandleSubmit, UseAccountBalance, UseFTBalance, UseFee, UseFTAllowance, FormattedValues } from '../../types';
import { AmountInput } from '../amount-input';
import { Balance } from '../balance';
import { Settings } from '../settings';
import { SubmitProgressBar } from '../submit-progress-bar';
import { SwapNetworkButton } from '../swap-network-button';
import { Token } from '../token';
import { TransactionModal } from '../transaction-modal';

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

  const { bridgingFee, vftManagerFee, priorityFee, ...config } = useFee();
  const accountBalance = useAccountBalance();
  const ftBalance = useFTBalance(token?.address);
  const allowance = useFTAllowance(token?.address);

  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const isNetworkAccountConnected = (network.isVara && Boolean(account)) || (!network.isVara && ethAccount.isConnected);

  const { open: openEthWalletModal } = useAppKit();
  const [isSubstrateWalletModalOpen, openSubstrateWalletModal, closeSubstrateWalletModal] = useModal();

  const [transactionModal, setTransactionModal] = useState<
    Omit<ComponentProps<typeof TransactionModal>, 'renderProgressBar'> | undefined
  >();

  const [priority, setPriority] = useState<(typeof PRIORITY)[keyof typeof PRIORITY]>(PRIORITY.HIGH);
  const shouldPayPriorityFee = priority === PRIORITY.HIGH;
  const time = shouldPayPriorityFee ? '20 mins' : '1 hour';

  const [claimType, setClaimType] = useState<(typeof CLAIM_TYPE)[keyof typeof CLAIM_TYPE]>(CLAIM_TYPE.AUTO);
  const shouldPayBridgingFee = claimType === CLAIM_TYPE.AUTO;

  const varaSymbol = useVaraSymbol();

  const openTransactionModal = (values: FormattedValues, estimatedFees: bigint) => {
    if (!token || !destinationToken) throw new Error('Address is not defined');

    const amount = values.amount;
    const receiver = values.accountAddress;
    const isVaraNetwork = network.isVara;
    const source = token.address;
    const destination = destinationToken.address;
    const close = () => setTransactionModal(undefined);

    setTransactionModal({ isVaraNetwork, amount, source, destination, receiver, estimatedFees, time, close });
  };

  const { onSubmit, requiredBalance, ...submit } = useHandleSubmit({
    bridgingFee: bridgingFee.value,
    shouldPayBridgingFee,
    vftManagerFee: vftManagerFee?.value,
    priorityFee: priorityFee?.value,
    shouldPayPriorityFee,
    allowance: allowance.data,
    accountBalance: accountBalance.data,
    onTransactionStart: openTransactionModal,
  });

  const isLoading =
    submit.isPending || accountBalance.isLoading || ftBalance.isLoading || config.isLoading || allowance.isLoading;

  const { form, amount, handleSubmit, setMaxBalance } = useSwapForm({
    accountBalance: accountBalance.data,
    ftBalance: ftBalance.data,
    onSubmit,
    requiredBalance,
  });

  const renderFromBalance = () => {
    const balance = token?.isNative ? accountBalance : ftBalance;

    return (
      <Balance
        value={balance.data}
        decimals={token?.decimals}
        symbol={token?.displaySymbol}
        isLoading={balance.isLoading}
        onMaxButtonClick={setMaxBalance}
      />
    );
  };

  const isEnoughBalance = () => {
    if (!api || isUndefined(bridgingFee.value) || !accountBalance.data) return false;

    let minBalance = shouldPayBridgingFee ? bridgingFee.value : 0n;

    if (network.isVara) {
      if (isUndefined(vftManagerFee?.value) || isUndefined(priorityFee?.value)) return false;

      minBalance += vftManagerFee.value + api.existentialDeposit.toBigInt();
      if (shouldPayPriorityFee) minBalance += priorityFee.value;
    }

    return accountBalance.data > minBalance;
  };

  const getButtonText = () => {
    if (!isEnoughBalance()) return `Not Enough ${network.isVara ? varaSymbol : 'ETH'}`;

    return requiredBalance.data ? 'Confirm Transfer' : 'Transfer';
  };

  const handleConnectWalletButtonClick = () => {
    const openWalletModal = network.isVara ? openSubstrateWalletModal : openEthWalletModal;

    void openWalletModal();
  };

  const handlePriorityChange = (value: typeof priority) => {
    requiredBalance.reset();
    setPriority(value);
  };

  const handleClaimTypeChange = (value: typeof claimType) => {
    requiredBalance.reset();
    setClaimType(value);
  };

  const renderTokenPrice = () => <TokenPrice symbol={token?.symbol} amount={amount} className={styles.price} />;
  const renderProgressBar = () => <SubmitProgressBar isVaraNetwork={network.isVara} {...submit} />;

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
                  symbol={token?.displaySymbol}
                  networkText={network.isVara ? 'Vara Testnet' : 'Ethereum Hoodi'}
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
                  symbol={destinationToken?.displaySymbol}
                  networkText={network.isVara ? 'Ethereum Hoodi' : 'Vara Testnet'}
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

          <Settings
            priority={priority}
            claimType={claimType}
            onPriorityChange={handlePriorityChange}
            onClaimTypeChange={handleClaimTypeChange}
            isVaraNetwork={network.isVara}
            fee={requiredBalance?.data?.fees}
            isFeeLoading={requiredBalance?.isPending}
            disabled={isLoading}
            time={time}
          />

          {isNetworkAccountConnected ? (
            <Button type="submit" text={getButtonText()} disabled={!isEnoughBalance()} isLoading={isLoading} block />
          ) : (
            <Button text="Connect Wallet" onClick={handleConnectWalletButtonClick} block />
          )}
        </form>
      </FormProvider>

      {isSubstrateWalletModalOpen && <WalletModal close={closeSubstrateWalletModal} />}

      {/* passing renderProgressBar explicitly to avoid state closure */}
      {transactionModal && <TransactionModal renderProgressBar={renderProgressBar} {...transactionModal} />}
    </>
  );
}

export { SwapForm };
