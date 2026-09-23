export type Env = {
  DB: D1Database;
  FILES: R2Bucket;
  /** HMAC-SHA256 secret for session tokens. Set via .dev.vars (local) or `wrangler secret` (prod). */
  JWT_SECRET: string;
  /** Comma-separated CORS origin allowlist, from [vars]. */
  CORS_ORIGINS: string;
};

export type AppEnv = {
  Bindings: Env;
  Variables: { userId: string };
};
