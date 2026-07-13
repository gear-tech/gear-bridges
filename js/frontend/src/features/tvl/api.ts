import { fetchWithGuard } from '@/utils';

const TVL_API_URL = 'https://api.llama.fi/tvl/vara-bridge';
const PROTOCOL_URL = 'https://defillama.com/protocol/vara-bridge';
const CHART_URL = 'https://defillama.com/chart/protocol/vara-bridge?theme=dark';

const getCurrentTvl = () => fetchWithGuard<number>({ url: TVL_API_URL });

export { CHART_URL, PROTOCOL_URL, TVL_API_URL, getCurrentTvl };
