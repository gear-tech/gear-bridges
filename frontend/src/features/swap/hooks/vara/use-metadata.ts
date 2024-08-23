import { ProgramMetadata } from '@gear-js/api';
import { useQuery } from '@tanstack/react-query';

function useMetadata(url: string | undefined) {
  const getMetadata = async () => {
    if (!url) throw new Error('Metadata URL is not found');

    const response = await fetch(url);
    const text = await response.text();

    return ProgramMetadata.from(`0x${text}`);
  };

  const { data } = useQuery({
    queryKey: ['metadata', url],
    queryFn: getMetadata,
    enabled: Boolean(url),
  });

  return data;
}

export { useMetadata };
