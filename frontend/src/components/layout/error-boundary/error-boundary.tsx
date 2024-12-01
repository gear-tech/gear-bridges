import { Button } from '@gear-js/vara-ui';
import { Component, ReactNode } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { useChangeEffect } from '@/hooks';

import { Container } from '../container';

import styles from './error-boundary.module.scss';

type Props = {
  children: ReactNode;
};

type FallbackProps = {
  message: string;
  reset: () => void;
};

type State = {
  error: Error | null;
};

function Fallback({ message, reset }: FallbackProps) {
  const { pathname } = useLocation();
  const navigate = useNavigate();

  useChangeEffect(() => {
    reset();
  }, [pathname]);

  return (
    <Container>
      <h2 className={styles.heading}>Oops! Something went wrong:</h2>
      <p className={styles.error}>{message}</p>

      <Button text="Go Back" size="small" onClick={() => navigate(-1)} />
    </Container>
  );
}

class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  reset = () => {
    this.setState({ error: null });
  };

  render() {
    if (!this.state.error) return this.props.children;

    return <Fallback message={this.state.error.message} reset={this.reset} />;
  }
}

export { ErrorBoundary };
