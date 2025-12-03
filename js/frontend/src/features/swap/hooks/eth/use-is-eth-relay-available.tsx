import { HexString } from '@gear-js/api';
import { getSlotByBlockNumber } from '@gear-js/bridge';
import { useAlert, useProgram, useProgramQuery } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';
import { useEffect } from 'react';
import { usePublicClient } from 'wagmi';

import { useNetworkType } from '@/context/network-type';
import { isUndefined, logger } from '@/utils';

import { CheckpointClientProgram, EthEventsProgram, HistoricalProxyProgram } from '../../consts';
import { useHistoricalProxyContractAddress } from '../vara';

function useErrorLoggingQuery<T>(query: T & { error: Error | null }, errorName: string) {
  const alert = useAlert();

  const { error } = query;

  useEffect(() => {
    if (!error) return;

    logger.error(errorName, error);
    alert.error(error.message);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [error]);

  return query;
}

function useSlot(blockNumber: bigint) {
  const { NETWORK_PRESET } = useNetworkType();
  const publicClient = usePublicClient();

  const query = useQuery({
    queryKey: [
      'slotByBlockNumber',
      NETWORK_PRESET.ETH_BEACON_NODE_ADDRESS,
      publicClient?.chain.id,
      blockNumber.toString(),
    ],

    queryFn: () => getSlotByBlockNumber(NETWORK_PRESET.ETH_BEACON_NODE_ADDRESS, publicClient!, blockNumber),
    enabled: Boolean(publicClient),
  });

  return useErrorLoggingQuery(query, 'Get slot by block number');
}

function useEthEventsContractAddress(slot: number | undefined) {
  const { data: historicalProxyContractAddress } = useHistoricalProxyContractAddress();

  const { data: historicalProxyProgram } = useProgram({
    id: historicalProxyContractAddress,
    library: HistoricalProxyProgram,
  });

  const query = useProgramQuery({
    program: historicalProxyProgram,
    serviceName: 'historicalProxy',
    functionName: 'endpointFor',
    args: [slot!],
    query: { enabled: !isUndefined(slot), select: (data) => ('ok' in data ? data.ok : undefined) },
  });

  return useErrorLoggingQuery(query, 'Get EthEvents contract address');
}

function useCheckpointClientContractAddress(ethEventsContractAddress: HexString | undefined) {
  const { data: ethEventsProgram } = useProgram({
    id: ethEventsContractAddress,
    library: EthEventsProgram,
  });

  const query = useProgramQuery({
    program: ethEventsProgram,
    serviceName: 'ethereumEventClient',
    functionName: 'checkpointLightClientAddress',
    args: [],
  });

  return useErrorLoggingQuery(query, 'Get CheckpointClient contract address');
}

function useIsEthRelayAvailable(blockNumber: bigint) {
  const { data: slot } = useSlot(blockNumber);
  const { data: ethEventsContractAddress } = useEthEventsContractAddress(slot);
  const { data: checkpointClientContractAddress } = useCheckpointClientContractAddress(ethEventsContractAddress);

  const { data: checkpointClientProgram } = useProgram({
    id: checkpointClientContractAddress,
    library: CheckpointClientProgram,
  });

  const query = useProgramQuery({
    program: checkpointClientProgram,
    serviceName: 'serviceCheckpointFor',
    functionName: 'get',
    args: [slot!],
    query: { enabled: !isUndefined(slot), select: (data) => 'ok' in data },
  });

  return useErrorLoggingQuery(query, 'Check if Eth relay is available');
}

export { useIsEthRelayAvailable };
