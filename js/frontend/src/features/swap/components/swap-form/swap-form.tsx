import { useAccount, useApi } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';
import { ComponentProps, useState } from 'react';
import { FormProvider } from 'react-hook-form';

import { Input } from '@/components';
import { useNetworkType } from '@/context';
import { TokenPrice } from '@/features/token-price';
import { useEthAccount, useModal, useVaraSymbol } from '@/hooks';
import { definedAssert, isUndefined } from '@/utils';

import PlusSVG from '../../assets/plus.svg?react';
import { CLAIM_TYPE, FIELD_NAME, NETWORK, PRIORITY } from '../../consts';
import { useBridgeContext } from '../../context';
import { useSwapForm } from '../../hooks';
import { UseSendTxs, UseAccountBalance, UseFTBalance, UseFee, FormattedValues, UseTxsEstimate } from '../../types';
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
  useSendTxs: UseSendTxs;
  useTxsEstimate: UseTxsEstimate;
  useFee: UseFee;
};

function SwapForm({ useAccountBalance, useFTBalance, useFee, useSendTxs, useTxsEstimate }: Props) {
  const { NETWORK_PRESET } = useNetworkType();
  const { network, token, destinationToken } = useBridgeContext();

  const { api } = useApi();

  const { bridgingFee, vftManagerFee, priorityFee, ...config } = useFee();
  const accountBalance = useAccountBalance();
  const ftBalance = useFTBalance(token?.address);

  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const isNetworkAccountConnected = (network.isVara && Boolean(account)) || (!network.isVara && ethAccount.isConnected);

  const { open: openEthWalletModal } = useAppKit();
  const [isSubstrateWalletModalOpen, openSubstrateWalletModal, closeSubstrateWalletModal] = useModal();

  const [transactionModal, setTransactionModal] = useState<
    Omit<ComponentProps<typeof TransactionModal>, 'renderProgressBar'> | undefined
  >();

  const [priority, setPriority] = useState<(typeof PRIORITY)[keyof typeof PRIORITY]>(PRIORITY.DEFAULT);
  const shouldPayPriorityFee = priority === PRIORITY.HIGH;
  const time = shouldPayPriorityFee || !network.isVara ? '20 mins' : '1 hour';

  const [claimType, setClaimType] = useState<(typeof CLAIM_TYPE)[keyof typeof CLAIM_TYPE]>(CLAIM_TYPE.AUTO);
  const shouldPayBridgingFee = claimType === CLAIM_TYPE.AUTO;

  const varaSymbol = useVaraSymbol();

  const { form, amount, formattedValues, handleSubmit, setMaxBalance } = useSwapForm({
    shouldPayBridgingFee,
    accountBalance: accountBalance.data,
    ftBalance: ftBalance.data,
  });

  const txsEstimate = useTxsEstimate({
    formValues: formattedValues,
    bridgingFee: bridgingFee.value,
    shouldPayBridgingFee,
    priorityFee: priorityFee?.value,
    shouldPayPriorityFee,
    vftManagerFee: vftManagerFee?.value,
    ftBalance: ftBalance.data,
  });

  const openTransactionModal = (values: FormattedValues) => {
    definedAssert(token, 'Token');
    definedAssert(destinationToken, 'Destination token');
    definedAssert(txsEstimate.data, 'Transaction estimation');

    setTransactionModal({
      amount: values.amount,
      receiver: values.accountAddress,
      isVaraNetwork: network.isVara,
      source: token.address,
      destination: destinationToken.address,
      estimatedFees: txsEstimate.data.fees,
      time,
      close: () => setTransactionModal(undefined),
    });
  };

  const sendTxs = useSendTxs({
    bridgingFee: bridgingFee.value,
    shouldPayBridgingFee,
    priorityFee: priorityFee?.value,
    shouldPayPriorityFee,
    vftManagerFee: vftManagerFee?.value,
    ftBalance: ftBalance.data,
    onTransactionStart: openTransactionModal,
  });

  const isLoading =
    sendTxs.isPending || accountBalance.isLoading || ftBalance.isLoading || config.isLoading || txsEstimate.isLoading;

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
    if (!api || !token || isUndefined(bridgingFee.value) || !txsEstimate.data || !accountBalance.data) return false;

    return accountBalance.data > txsEstimate.data.requiredBalance;
  };

  const getButtonText = () => {
    if (!txsEstimate.data) return 'Fill the form';
    if (!isEnoughBalance()) return `Not Enough ${network.isVara ? varaSymbol : 'ETH'}`;

    return 'Transfer';
  };

  const handleConnectWalletButtonClick = () => {
    const openWalletModal = network.isVara ? openSubstrateWalletModal : openEthWalletModal;

    void openWalletModal();
  };

  const renderTokenPrice = () => <TokenPrice symbol={token?.symbol} amount={amount} className={styles.price} />;
  const renderProgressBar = () => <SubmitProgressBar isVaraNetwork={network.isVara} {...sendTxs} />;

  return (
    <>
      <FormProvider {...form}>
        <form onSubmit={handleSubmit(sendTxs.mutateAsync)} className={styles.form}>
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
                  networkText={network.isVara ? NETWORK_PRESET.NETWORK_NAME.VARA : NETWORK_PRESET.NETWORK_NAME.ETH}
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
                  networkText={network.isVara ? NETWORK_PRESET.NETWORK_NAME.ETH : NETWORK_PRESET.NETWORK_NAME.VARA}
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
            onPriorityChange={setPriority}
            onClaimTypeChange={setClaimType}
            isVaraNetwork={network.isVara}
            fee={txsEstimate.data?.fees}
            isFeeLoading={txsEstimate.isLoading}
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
