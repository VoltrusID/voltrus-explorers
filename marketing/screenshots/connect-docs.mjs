// Voltrus Connect screenshots for the website (voltrus.id/voltrus-connect/ and the tool pages): the real Connect dialog
// of Modbus Poll and BACnet Explorer driven against a small in-page Connect server, so every state is rendered by the
// app itself (sign-in prompt, Account, Profiles, Teams, Remote, the save flow, and the same profiles on an iPhone).
//
//   bun marketing/screenshots/render.mjs modbus-poll --bare connect-signin connect-account connect-save connect-profiles connect-teams connect-remote connect-remote-join
//   bun marketing/screenshots/render.mjs modbus-poll --device=iphone --bare connect-signin connect-profiles
//
// The scenes are flagged `review`, so the regular marketing render skips them unless they are named.
// Demo data only: example.com addresses, no real people or sites.

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export const TEAM = { id: "team-plant", name: "Plant maintenance" };
const ME = { id: "00000000-0000-4000-8000-0000000000a1", email: "dewi@example.com" };

const SITES = [
  {
    id: "site-cikarang",
    name: "Cikarang plant",
    team_id: TEAM.id,
    login_server: "https://remote.example.com",
    dashboard_url: "https://remote.example.com/admin",
    subnets: ["192.168.10.0/24", "192.168.20.0/24"],
    notes: "Gateway in the MCC room. Ask the plant electrician for a key.",
  },
  {
    id: "site-surabaya",
    name: "Surabaya cold store",
    team_id: null,
    login_server: "https://remote.example.com",
    dashboard_url: null,
    subnets: ["10.20.0.0/24"],
    notes: "Chiller controllers and meters",
  },
];

/** Runs in the page after the Tauri mock: a stateful stand-in for the Connect server behind the connect_* commands. */
function installConnectMock(cfg) {
  const base = window.__TAURI_INTERNALS__.invoke;
  const now = Date.now();
  const me = {
    user: { id: cfg.me.id, email: cfg.me.email, email_verified: true },
    entitlement: { active: true, source: "console", expires_at: now + 296 * 86400000 },
    teams: [{ id: cfg.team.id, name: cfg.team.name, role: "owner" }],
    app_account_token: null,
  };
  let seq = 0;
  const server = cfg.serverProfiles.map((p, i) => ({
    id: `00000000-0000-4000-8000-00000000b${i}`,
    app: cfg.app,
    name: p.name,
    team_id: p.shared ? cfg.team.id : null,
    data: p.data,
    version: 1,
    seq: ++seq,
    deleted: false,
    can_edit: true,
    updated_at: now - (i + 1) * 3600000,
  }));
  const teams = [
    {
      ...me.teams[0],
      members: [
        { user_id: cfg.me.id, email: cfg.me.email, role: "owner" },
        { user_id: "m2", email: "andi@example.com", role: "member" },
        { user_id: "m3", email: "sari@example.com", role: "member" },
        { user_id: "m4", email: "rudi@example.com", role: "member" },
      ],
      invites: [{ id: "i1", email: "budi@example.com", expires_at: now + 6 * 86400000 }],
    },
  ];
  const sync = ({ request }) => {
    const ops = [];
    const used = new Set();
    for (const l of request.locals) {
      const known = (l.link && server.find((s) => s.id === l.link.id)) || server.find((s) => s.name === l.name);
      if (known) used.add(known.id);
      if (l.link && !l.dirty) continue;
      let s = known;
      if (s) {
        s.data = l.data;
        s.version += 1;
      } else {
        s = {
          id: `00000000-0000-4000-8000-00000000c${server.length}`,
          app: cfg.app,
          name: l.name,
          team_id: l.team_id,
          data: l.data,
          version: 1,
          seq: ++seq,
          deleted: false,
          can_edit: true,
          updated_at: Date.now(),
        };
        server.push(s);
        used.add(s.id);
      }
      ops.push({ op: "saved", key: l.key, profile: { ...s } });
    }
    if (window.__pull && server.length > 0) {
      window.__pull = false;
      const s = server[0];
      s.version += 1;
      s.data = { ...s.data, tcp: { ...s.data.tcp, pollInterval: 250 } };
      ops.push({ op: "pulled", key: s.name, profile: { ...s } });
      used.add(s.id);
    }
    if (request.full) {
      const linked = new Set(request.locals.filter((l) => l.link).map((l) => l.link.id));
      for (const s of server) if (!used.has(s.id) && !linked.has(s.id)) ops.push({ op: "added", profile: { ...s } });
    }
    return { ops, cursor: seq };
  };
  const commands = {
    connect_info: () => ({ app_slug: cfg.app, store: false, console_url: "https://console.voltrus.id/app/connect", delete_account_url: "https://console.voltrus.id/app/settings?delete=1" }),
    connect_signed_in: () => cfg.signedIn,
    connect_me: () => me,
    connect_sync: sync,
    connect_open_url: () => null,
    connect_sign_in: () => null,
    connect_sign_out: () => null,
    connect_new_id: () => crypto.randomUUID(),
    connect_teams: () => teams,
    connect_remote_sites: () => cfg.sites,
    connect_remote_join: ({ siteId }) => {
      const s = cfg.sites.find((x) => x.id === siteId);
      const args = ["tailscale", "up", `--login-server=${s.login_server}`, "--authkey=<ENROLMENT_KEY>", "--accept-routes"];
      return {
        site_id: s.id,
        name: s.name,
        login_server: s.login_server,
        needs_enrolment_key: true,
        tailscale_command: args.join(" "),
        steps: [
          "Install the Tailscale client on this computer.",
          "Ask an admin of this Voltrus Remote install for an engineer-laptop enrolment key (Remote dashboard: Add engineer laptop).",
          "Run the command with the key in place of <ENROLMENT_KEY>.",
          `Once the gateway's routes are approved, reach the PLC network (${s.subnets.join(", ")}) directly.`,
        ],
        limits: [
          "Voltrus cannot issue this key: only an admin of the install holds the control-api token.",
          "Enrolment keys are single-use and expire (30 minutes by default).",
          "A gateway's subnet route only works after an install admin approves it.",
          "The tunnel suits engineering access and polling, not real-time control loops.",
        ],
      };
    },
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

export const connectMockInit = (cfg) => `(${installConnectMock.toString()})(${JSON.stringify({ me: ME, team: TEAM, sites: SITES, ...cfg })});`;

async function openDialog(page) {
  await page.waitForSelector('button[aria-label^="Voltrus Connect"]', { timeout: 10000 });
  await page.click('button[aria-label^="Voltrus Connect"]');
  await page.waitForSelector(".vt-dialog, [role=dialog]", { timeout: 10000 });
  await sleep(300);
}

async function clickTab(page, label) {
  const ok = await page.evaluate((l) => {
    const t = [...document.querySelectorAll('[role="tab"], button')].find((el) => el.textContent.trim().startsWith(l) && el.closest(".vt-dialog, [role=dialog]"));
    if (!t) return false;
    t.click();
    return true;
  }, label);
  if (!ok) throw new Error(`tab ${label} not found`);
  await sleep(400);
}

async function waitText(page, text) {
  await page.waitForFunction((t) => document.body.textContent.includes(t), { timeout: 15000 }, text);
}

async function clickText(page, selector, text) {
  const ok = await page.evaluate(
    (s, t) => {
      const el = [...document.querySelectorAll(s)].find((e) => e.textContent.trim() === t);
      if (!el) return false;
      el.click();
      return true;
    },
    selector,
    text,
  );
  if (!ok) throw new Error(`${selector} "${text}" not found`);
}

/**
 * Scene builders shared by both apps. `serverProfiles` are what the account already holds; `prepare` gets the app to the
 * state the scene starts from (e.g. a connected device); `saveFlow` runs the app's own way of saving a site.
 */
export function connectDocScenes({ app, pre = "", serverProfiles, prepare, saveFlow }) {
  const signedIn = { app, serverProfiles, signedIn: true };
  const base = (id, headline, init, setup, extra = {}) => ({ id, review: true, headline, sub: "Voltrus Connect", init: `${pre}\n${init}`, settleMs: 500, setup, crop: ".vt-dialog", ...extra });
  return [
    base("connect-signin", "Sign in", connectMockInit({ app, serverProfiles, signedIn: false }), async (page) => {
      await prepare?.(page);
      await openDialog(page);
      await waitText(page, "Sign in with Voltrus");
    }),
    base("connect-account", "Account", connectMockInit(signedIn), async (page) => {
      await prepare?.(page);
      // A change made on another device arrives with the sync the dialog starts when it opens.
      await page.evaluate(() => (window.__pull = true));
      await openDialog(page);
      await waitText(page, "Last synced");
    }),
    base(
      "connect-save",
      "Save a site",
      connectMockInit(signedIn),
      async (page) => {
        await prepare?.(page);
        await saveFlow(page, { openDialog, clickTab, waitText, clickText, sleep });
      },
      { crop: undefined },
    ),
    base("connect-profiles", "Profiles", connectMockInit(signedIn), async (page) => {
      await prepare?.(page);
      await openDialog(page);
      await waitText(page, "Last synced");
      await clickTab(page, "Profiles");
      await waitText(page, "Synced");
    }),
    base("connect-teams", "Teams", connectMockInit(signedIn), async (page) => {
      await prepare?.(page);
      await openDialog(page);
      await waitText(page, "Last synced");
      await clickTab(page, "Teams");
      await waitText(page, "andi@example.com");
    }),
    base("connect-remote", "Remote", connectMockInit(signedIn), async (page) => {
      await prepare?.(page);
      await openDialog(page);
      await waitText(page, "Last synced");
      await clickTab(page, "Remote");
      await waitText(page, "Cikarang plant");
    }),
    base("connect-remote-join", "How to join", connectMockInit(signedIn), async (page) => {
      await prepare?.(page);
      await openDialog(page);
      await waitText(page, "Last synced");
      await clickTab(page, "Remote");
      await waitText(page, "Cikarang plant");
      await clickText(page, "button", "How to join");
      await waitText(page, "Join Cikarang plant");
    }),
  ];
}

/** iPhone / iPad scenes: a device that has no profiles of its own receives them from the account. */
export function connectDocIosScenes({ app, serverProfiles, init, prepare }) {
  const mk = (id, signed, setup) => ({
    id,
    review: true,
    headline: "Voltrus Connect",
    sub: "Voltrus Connect",
    init: `${init}\n${connectMockInit({ app, serverProfiles, signedIn: signed })}`,
    settleMs: 500,
    setup,
  });
  return [
    mk("connect-signin", false, async (page, device) => {
      await prepare?.(page, device);
      await openDialog(page);
      await waitText(page, "Sign in with Voltrus");
    }),
    mk("connect-profiles", true, async (page, device) => {
      await prepare?.(page, device);
      await openDialog(page);
      await waitText(page, "Last synced");
      await clickTab(page, "Profiles");
      await waitText(page, "Synced");
    }),
  ];
}
