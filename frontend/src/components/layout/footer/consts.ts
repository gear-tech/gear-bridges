import DiscordSVG from './discord.svg?react';
import GithubSVG from './github.svg?react';
import MediumSVG from './medium.svg?react';
import TelegramSVG from './telegram.svg?react';
import XSVG from './x.svg?react';
import YouTubeSVG from './youtube.svg?react';

const LIST = [
  {
    heading: 'LEARN',
    links: [
      { text: 'Network Tokenomics', href: 'https://wiki.vara.network/docs/tokenomics' },
      { text: 'Gear Wiki', href: 'https://wiki.gear-tech.io' },
      { text: 'Vara Wiki', href: 'https://wiki.vara.network' },
    ],
  },
  {
    heading: 'BUILD',
    links: [
      { text: 'Get VARA', href: 'https://vara.network/developers#get-vara' },
      { text: 'Implement smart contracts', href: 'https://wiki.vara.network/docs/build' },
      { text: 'Deploy', href: 'https://idea.gear-tech.io' },
      { text: 'Build React application', href: 'https://wiki.gear-tech.io/docs/api/getting-started' },
    ],
  },
  {
    heading: 'NETWORK',
    links: [
      { text: 'Become a Validator', href: 'https://wiki.vara.network/docs/staking/validate' },
      { text: 'Staking', href: 'https://wiki.vara.network/docs/staking/overview' },
      { text: 'Join Ambassador program', href: 'https://vara.network/ambassadors/apply' },
      { text: 'Vara NFT', href: 'https://nft-marketplace.vara.network' },
      { text: 'Press Kit', href: 'https://vara.network/press-kit' },
    ],
  },
  {
    heading: 'INSPECT',
    links: [
      { text: 'Node Telemetry', href: 'https://telemetry.rs' },
      { text: 'Vara explorer (Subscan)', href: 'https://vara.subscan.io' },
    ],
  },
];

const SOCIALS = [
  {
    SVG: XSVG,
    href: 'https://twitter.com/VaraNetwork',
  },
  {
    SVG: GithubSVG,
    href: 'https://github.com/gear-foundation',
  },
  {
    SVG: DiscordSVG,
    href: 'https://discord.gg/x8ZeSy6S6K',
  },
  {
    SVG: MediumSVG,
    href: 'https://medium.com/@VaraNetwork',
  },
  {
    SVG: YouTubeSVG,
    href: 'https://www.youtube.com/@Gear_Foundation',
  },
  {
    SVG: TelegramSVG,
    href: 'https://t.me/VaraNetwork_Global',
  },
];

export { LIST, SOCIALS };
