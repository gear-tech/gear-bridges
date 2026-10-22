import '@gear-js/vara-ui/dist/style.css';
import React from 'react';
import ReactDOM from 'react-dom/client';

import './index.scss';

// Deployment configuration is evaluated before providers can mount an error boundary.
void import('./bootstrap').catch(() => {
  ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
      <main>
        <p role="status">
          HOLD: frontend deployment configuration is unavailable. No transfer or claim can be submitted.
        </p>
      </main>
    </React.StrictMode>,
  );
});
