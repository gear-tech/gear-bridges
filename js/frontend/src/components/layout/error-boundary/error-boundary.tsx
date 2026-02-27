import { Button } from '@gear-js/vara-ui';
import { ErrorBoundary as SentryErrorBoundary, FallbackRender } from '@sentry/react';
import { ComponentProps, PropsWithChildren } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { useChangeEffect } from '@/hooks';

import { Container } from '../container';

import styles from './error-boundary.module.scss';

// eslint-disable-next-line @typescript-eslint/unbound-method
function Fallback({ error, resetError }: ComponentProps<FallbackRender>) {
  const { pathname } = useLocation();
  const navigate = useNavigate();

  useChangeEffect(() => {
    resetError();
  }, [pathname]);

  return (
    <Container>
      <h2 className={styles.heading}>Oops! Something went wrong:</h2>
      <p className={styles.error}>{error instanceof Error ? error.message : String(error)}</p>

      <Button text="Go Back" size="small" onClick={() => navigate(-1)} />
    </Container>
  );
}

function ErrorBoundary({ children }: PropsWithChildren) {
  return <SentryErrorBoundary fallback={Fallback}>{children}</SentryErrorBoundary>;
}

export { ErrorBoundary };
