import { QueryKey, useQueryClient } from '@tanstack/react-query';
import { useWatchBlockNumber } from 'wagmi';

function useInvalidateOnBlock({ queryKey, enabled }: { queryKey: QueryKey; enabled?: boolean }) {
  const queryClient = useQueryClient();

  const onBlockNumber = () => void queryClient.invalidateQueries({ queryKey }, { cancelRefetch: false });

  useWatchBlockNumber({ enabled, onBlockNumber });
}

export { useInvalidateOnBlock };
