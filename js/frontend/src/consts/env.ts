const SENTRY_DSN = import.meta.env.VITE_SENTRY_DSN as string | undefined;
const GTM_ID = import.meta.env.VITE_GTM_ID as string | undefined;

export { SENTRY_DSN, GTM_ID };
