import { Container } from '@/components';
import { QUESTIONS, Accordion } from '@/features/faq';

import styles from './faq.module.scss';

function FAQ() {
  const renderQuestions = () =>
    QUESTIONS.map(({ question, answer }, index) => (
      <li key={index}>
        <Accordion heading={question}>{answer}</Accordion>
      </li>
    ));

  return (
    <Container maxWidth="724px" className={styles.container}>
      <h1 className={styles.heading}>Vara Network Bridge</h1>
      <p className={styles.subheading}>
        A bridge system enabling the transfer of wrapped tokens between Vara Network and Ethereum mainnet.
      </p>

      <ul className={styles.questions}>{renderQuestions()}</ul>
    </Container>
  );
}

export { FAQ };
