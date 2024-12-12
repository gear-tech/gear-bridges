import { useEffect } from 'react';

function useLargeModal() {
  useEffect(() => {
    // TODO: monkey patch, update after @gear-js/vara-ui is updated to support different modal sizes
    setTimeout(() => {
      const modalElement = document.querySelector('#modal-root > div > div') as HTMLElement;

      if (modalElement) {
        modalElement.style.maxWidth = '680px';
      }
    }, 0);
  }, []);
}

export { useLargeModal };
