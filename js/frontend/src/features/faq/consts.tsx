import ArrowsDoubleSVG from './assets/arrows-double.svg?react';
import styles from './components/accordion/accordion.module.scss';

const QUESTIONS = [
  {
    question: (
      <>
        What is the Vara <ArrowsDoubleSVG className={styles.inlineIcon} /> Ethereum Bridge?
      </>
    ),
    answer: (
      <p>
        The bridge allows you to move tokens between <span className={styles.bold}>Vara Network</span> and{' '}
        <span className={styles.bold}>Ethereum</span>. When you bridge tokens, they are locked or burned on one network
        and minted or unlocked on the other.
      </p>
    ),
  },

  {
    question: 'What tokens can I bridge?',
    answer: (
      <>
        <p>You can bridge the following tokens:</p>

        <ul>
          <li>
            <span className={styles.bold}>VARA</span> <ArrowsDoubleSVG className={styles.inlineIcon} />{' '}
            <span className={styles.bold}>wVARA</span>
          </li>
          <li>
            <span className={styles.bold}>ETH</span> <ArrowsDoubleSVG className={styles.inlineIcon} />{' '}
            <span className={styles.bold}>wETH</span>
          </li>
          <li>
            <span className={styles.bold}>USDT</span> <ArrowsDoubleSVG className={styles.inlineIcon} />{' '}
            <span className={styles.bold}>wUSDT</span>
          </li>
          <li>
            <span className={styles.bold}>USDC</span> <ArrowsDoubleSVG className={styles.inlineIcon} />{' '}
            <span className={styles.bold}>wUSDC</span>
          </li>
        </ul>
      </>
    ),
  },

  {
    question: 'How does the bridging process work?',
    answer: (
      <ul>
        <li>
          <span className={styles.bold}>From Vara to Ethereum:</span> Your tokens are locked on Vara and an equivalent
          amount of wrapped tokens is minted on Ethereum.
        </li>
        <li>
          <span className={styles.bold}>From Ethereum to Vara:</span> Your wrapped tokens are burned on Ethereum and
          your original tokens are unlocked on Vara.
        </li>
      </ul>
    ),
  },

  {
    question: 'How long does bridging take?',
    answer: (
      <p>
        Bridging usually takes around <span className={styles.bold}>20 minutes</span>, depending on the finalization
        times of the networks and current network conditions.
      </p>
    ),
  },

  {
    question: 'What fees do I need to pay?',
    answer: (
      <>
        <ul>
          <li>
            When bridging from <span className={styles.bold}>Ethereum</span>, you must pay{' '}
            <span className={styles.bold}>gas fees in ETH.</span>
          </li>
          <li>
            When bridging from <span className={styles.bold}>Vara</span>, you pay{' '}
            <span className={styles.bold}>fees in VARA.</span>
          </li>
        </ul>

        <p>All applicable fees will be displayed before you confirm your transaction.</p>
      </>
    ),
  },

  {
    question: 'What wallets are supported?',
    answer: (
      <>
        <p>
          You can connect using: <span className={styles.bold}>MetaMask</span> and other{' '}
          <span className={styles.bold}>Ethereum-compatible wallets</span> for Ethereum.
        </p>
        <p>
          <span className={styles.bold}>Substrate-compatible wallets</span> (such as{' '}
          <span className={styles.bold}>SubWallet</span>, <span className={styles.bold}>Polkadot.js extension</span> or
          other Vara-supported wallets) for Vara.
        </p>
      </>
    ),
  },

  {
    question: 'Why do I see "wrapped" tokens after bridging?',
    answer: (
      <>
        <p>
          Wrapped tokens (e.g., <span className={styles.bold}>wVARA</span>, <span className={styles.bold}>wETH</span>)
          represent your original assets 1:1 and can be freely used across the Ethereum ecosystem.
        </p>

        <p>You can always bridge them back to Vara to redeem your original tokens.</p>
      </>
    ),
  },

  {
    question: 'What happens if my transaction seems stuck?',
    answer: (
      <>
        <p>
          Sometimes after signing the first transaction (locking your tokens), the second required signature (for the
          payment fee) might be skipped.
        </p>

        <p>In this case:</p>

        <ul>
          <li>
            <span className={styles.bold}>Tokens will be locked, but not yet bridged.</span>
          </li>
          <li>
            <span className={styles.bold}>The UI will notify you</span> and allow you to sign the pending payment
            transaction.
          </li>
          <li>Once you sign the payment, your locked tokens will be correctly transferred to the other network.</li>
        </ul>
      </>
    ),
  },

  {
    question: 'Will the app prevent mistakes like insufficient gas or unsupported tokens?',
    answer: (
      <p>
        Yes. If you don&apos;t have enough gas (ETH on Ethereum, VARA on Vara),{' '}
        <span className={styles.bold}>the UI will prevent you from starting the bridging process.</span>
      </p>
    ),
  },

  {
    question: 'Is the bridge secure?',
    answer: (
      <p>
        Yes. The bridge uses <span className={styles.bold}>zero-knowledge (ZK) proofs</span> to cryptographically verify
        every transfer. All operations are validated by smart contracts — ensuring security without relying on trust in
        any single party.
      </p>
    ),
  },

  {
    question: 'Can I bridge NFTs or other types of assets?',
    answer: (
      <>
        <ul>
          <li>
            <span className={styles.bold}>On the transport layer:</span> Technically yes — the bridge enables
            transmission of arbitrary data between applications running on Vara network and Ethereum.
          </li>
          <li>
            <span className={styles.bold}>On the bridging layer:</span> Currently, only fungible tokens (ERC-20 style)
            are supported in the UI.
          </li>
        </ul>

        <p>
          <span className={styles.bold}>Good news:</span> The bridge supports cross-chain communication with arbitrary
          data. Developer guides with integration examples:
        </p>

        <ul>
          <li>
            <a
              href="https://wiki.gear.foundation/docs/bridge/developer-guide-vara-eth"
              target="_blank"
              rel="noreferrer"
              className="faqLink">
              Vara <ArrowsDoubleSVG className={styles.inlineIcon} /> Ethereum
            </a>
          </li>
          <li>
            <a
              href="https://wiki.gear.foundation/docs/bridge/developer-guide-eth-vara"
              target="_blank"
              rel="noreferrer"
              className="faqLink">
              Ethereum <ArrowsDoubleSVG className={styles.inlineIcon} /> Vara
            </a>
          </li>
        </ul>

        <p>
          For questions or assistance, contact via official social media{' '}
          <a href="https://vara.network" target="_blank" rel="noreferrer">
            channels
          </a>
          .
        </p>
      </>
    ),
  },

  {
    question: 'Where can I find more technical details about the bridge?',
    answer: (
      <p>
        You can learn more about the architecture, security model, and integration options here:{' '}
        <a href="https://wiki.vara.network/docs/bridge" target="_blank" rel="noreferrer">
          Read the Bridge Wiki
        </a>
        .
      </p>
    ),
  },
];

export { QUESTIONS };
