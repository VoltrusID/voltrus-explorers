// App Store Connect review screenshot for the Voltrus Connect subscriptions: the signed-in, free account
// view of the Connect dialog with the StoreKit subscribe panel (App Store build, prices come from StoreKit).
//
//   bun marketing/screenshots/render.mjs <app> --bare connect-review
//
// Writes out/bare-<NN>-connect-review.png at 2560x1600 (16:10), an accepted Mac review screenshot size.
// The scene is flagged `review`, so the marketing render skips it unless it is named.

const PRICES = { monthly: "$4.99", yearly: "$39.99" };

/** Runs in the page after the Tauri mock: answers the connect_* commands as a free account in an App Store build. */
function installConnectMock(cfg) {
  const base = window.__TAURI_INTERNALS__.invoke;
  const me = {
    user: { id: "00000000-0000-4000-8000-000000000001", email: "app-review@voltrus.id", email_verified: true },
    entitlement: { active: false, source: "none", expires_at: null },
    teams: [],
    app_account_token: null,
  };
  const commands = {
    connect_info: () => ({ app_slug: cfg.app, store: true, console_url: null }),
    connect_signed_in: () => true,
    connect_me: () => me,
    connect_store_products: () => cfg.products,
    connect_sync: () => ({ ops: [], cursor: 0 }),
    connect_link_purchases: () => ({ linked: 0, me }),
    connect_open_url: () => null,
  };
  window.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
    if (cmd.startsWith("connect_")) {
      const handler = commands[cmd];
      if (!handler) throw new Error(`mock: unknown command ${cmd}`);
      return handler(args || {});
    }
    return base(cmd, args);
  };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** `productBase` is `pro_product_id` without `.pro` (see connect.json); ids are `<base>.connect.monthly|yearly`. */
export function connectReviewScene(app, productBase) {
  const products = [
    { id: `${productBase}.connect.monthly`, title: "Voltrus Connect", description: "Team sharing and Remote sites. Billed monthly.", price: PRICES.monthly },
    { id: `${productBase}.connect.yearly`, title: "Voltrus Connect", description: "Team sharing and Remote sites. Billed yearly.", price: PRICES.yearly },
  ];
  return {
    id: "connect-review",
    review: true,
    headline: "Voltrus Connect",
    sub: "Review screenshot",
    init: `(${installConnectMock.toString()})(${JSON.stringify({ app, products })});`,
    settleMs: 500,
    async setup(page) {
      await page.setViewport({ width: 1280, height: 800, deviceScaleFactor: 2 });
      await page.waitForSelector('button[aria-label^="Voltrus Connect"]', { timeout: 10000 });
      await page.click('button[aria-label^="Voltrus Connect"]');
      await page.waitForFunction(() => document.body.textContent.includes("Connect subscription"), { timeout: 10000 });
      await sleep(300);
    },
  };
}
