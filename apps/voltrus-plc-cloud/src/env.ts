export type Env = {
  DB: D1Database;
  FILES: R2Bucket;
  /** HMAC-SHA256 secret for session tokens. Set via .dev.vars (local) or `wrangler secret` (prod). */
  JWT_SECRET: string;
  /** Comma-separated CORS origin allowlist, from [vars]. */
  CORS_ORIGINS: string;
  /** RelayRoom Durable Object namespace, one room per device id. */
  RELAY: DurableObjectNamespace;
  /** Polar.sh webhook signing secret (whsec_…). Unset = webhook endpoint returns 501. */
  POLAR_WEBHOOK_SECRET?: string;
  /** Polar product ids that map a subscription to a plan; unknown products are ignored. */
  POLAR_PRODUCT_SOLO?: string;
  POLAR_PRODUCT_TEAM?: string;
  /** Polar organization access token (customer_sessions:write) for the billing portal link. Unset = portal returns 501. */
  POLAR_ACCESS_TOKEN?: string;
  /** Polar checkout URLs surfaced by GET /billing/plan. Unset/empty = that plan hides its Upgrade button. */
  POLAR_CHECKOUT_SOLO?: string;
  POLAR_CHECKOUT_TEAM?: string;
};

export type AppEnv = {
  Bindings: Env;
  Variables: { userId: string };
};
