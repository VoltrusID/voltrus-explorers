//! Profile sync: one pass reconciles the app's local profiles with the server.
//!
//! The app owns local storage. It hands over its profiles (with the link to the server copy and whether
//! they changed since the last sync) and applies the returned operations. Nothing here guesses at
//! deletions: a profile missing from a full list is not deleted (team profiles disappear while the
//! entitlement is inactive), only tombstones delete.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ConnectError;
use crate::profiles::{self, Profile, ProfileDraft, SaveOutcome, new_uuid};
use crate::session::Session;

#[derive(Debug, Clone, Deserialize)]
pub struct Link {
    pub id: String,
    pub version: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalEntry {
    /// Opaque, stable key the app uses for this profile.
    pub key: String,
    pub name: String,
    pub data: Value,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub link: Option<Link>,
    /// Name, data or team changed since the last successful sync.
    #[serde(default)]
    pub dirty: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SyncRequest {
    pub locals: Vec<LocalEntry>,
    /// Server ids of profiles the user deleted locally.
    #[serde(default)]
    pub deleted: Vec<String>,
    /// Cursor from the previous sync.
    #[serde(default)]
    pub cursor: Option<i64>,
    /// Also fetch the full visible list (first sync, or the teams/entitlement changed).
    #[serde(default)]
    pub full: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Upload accepted (created or updated): link `key` to `profile` and mark it clean.
    Saved { key: String, profile: Profile },
    /// The server copy is newer and the local one is untouched: replace the local one.
    Pulled { key: String, profile: Profile },
    /// A profile the app does not have yet.
    Added { profile: Profile },
    /// Deleted on another device and untouched here.
    Removed { key: String },
    /// Both sides changed. The app asks the user; `connect_profile_save` resolves it.
    Conflict { key: String, server: Option<Profile> },
    /// A local deletion reached the server.
    Deleted { id: String },
    Failed { key: String, message: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    pub ops: Vec<Op>,
    pub cursor: Option<i64>,
}

pub async fn run(session: &Session, app: &str, req: SyncRequest) -> Result<SyncResult, ConnectError> {
    let mut ops = Vec::new();

    // 1. What changed on the server.
    let mut cursor = req.cursor;
    let mut remote: HashMap<String, Profile> = HashMap::new();
    let mut tombstones: HashMap<String, Profile> = HashMap::new();
    if let Some(since) = req.cursor {
        let page = profiles::list(session, app, Some(since)).await?;
        cursor = page.cursor.or(cursor);
        for p in page.items {
            if p.deleted {
                tombstones.insert(p.id.clone(), p);
            } else {
                remote.insert(p.id.clone(), p);
            }
        }
    }
    if req.full || req.cursor.is_none() {
        let page = profiles::list(session, app, None).await?;
        if req.cursor.is_none() {
            cursor = page.cursor;
        }
        for p in page.items {
            remote.insert(p.id.clone(), p);
        }
    }

    // 2. Local deletions go first so a deleted profile is not pulled back in below.
    let mut deleted_ids = Vec::new();
    for id in &req.deleted {
        match profiles::delete(session, id).await {
            Ok(()) => {
                remote.remove(id);
                deleted_ids.push(id.clone());
                ops.push(Op::Deleted { id: id.clone() });
            }
            Err(e) => ops.push(Op::Failed { key: id.clone(), message: e.to_string() }),
        }
    }

    // 3. Each local profile.
    let mut linked_ids = HashMap::new();
    for local in &req.locals {
        if let Some(link) = &local.link {
            linked_ids.insert(link.id.clone(), local.key.clone());
        }
        match &local.link {
            None => upload(session, app, local, None, &mut ops).await?,
            Some(link) => {
                if tombstones.contains_key(&link.id) {
                    if local.dirty {
                        // Deleted elsewhere but edited here: keep the edit as a new profile.
                        upload(session, app, local, None, &mut ops).await?;
                    } else {
                        ops.push(Op::Removed { key: local.key.clone() });
                    }
                } else if local.dirty {
                    upload(session, app, local, Some(link), &mut ops).await?;
                } else if let Some(server) = remote.get(&link.id).filter(|s| s.version > link.version) {
                    ops.push(Op::Pulled { key: local.key.clone(), profile: server.clone() });
                }
            }
        }
    }

    // 4. Server profiles the app does not have.
    let mut added: Vec<&Profile> = remote.values().filter(|p| !linked_ids.contains_key(&p.id) && !deleted_ids.contains(&p.id)).collect();
    added.sort_by(|a, b| a.seq.cmp(&b.seq).then_with(|| a.id.cmp(&b.id)));
    ops.extend(added.into_iter().map(|p| Op::Added { profile: p.clone() }));

    Ok(SyncResult { ops, cursor })
}

/// Saves one local profile. `link: None` creates it under a fresh id.
async fn upload(session: &Session, app: &str, local: &LocalEntry, link: Option<&Link>, ops: &mut Vec<Op>) -> Result<(), ConnectError> {
    let fail = |ops: &mut Vec<Op>, e: ConnectError| ops.push(Op::Failed { key: local.key.clone(), message: e.to_string() });
    let draft = ProfileDraft {
        id: match link {
            Some(l) => l.id.clone(),
            None => new_uuid()?,
        },
        name: local.name.clone(),
        team_id: local.team_id.clone(),
        data: local.data.clone(),
        base_version: link.map_or(0, |l| l.version),
    };
    match profiles::save(session, app, &draft).await {
        Ok(SaveOutcome::Saved { profile }) => ops.push(Op::Saved { key: local.key.clone(), profile }),
        Ok(SaveOutcome::Conflict { server }) => ops.push(Op::Conflict { key: local.key.clone(), server }),
        // A sign-in problem ends the whole pass; anything else only fails this profile.
        Err(ConnectError::NotSignedIn) => return Err(ConnectError::NotSignedIn),
        Err(ConnectError::Network(m)) => return Err(ConnectError::Network(m)),
        Err(e) => fail(ops, e),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Api;
    use crate::session::TokenSet;
    use crate::testutil::{MockServer, Reply, json};
    use crate::vault::{MemoryVault, Vault};
    use parking_lot::Mutex;
    use serde_json::json;
    use std::sync::Arc;

    /// An in-memory stand-in for the profiles endpoints, with the server's version rules.
    #[derive(Default)]
    struct Fake {
        rows: Vec<Value>,
        seq: i64,
        /// Ids whose writes are refused with 402 (a team profile while the entitlement is inactive).
        refuse: Vec<String>,
        /// Ids hidden from lists (same reason).
        hidden: Vec<String>,
    }

    impl Fake {
        fn put(&mut self, id: &str, name: &str, data: Value, version: i64) {
            self.seq += 1;
            self.rows.retain(|r| r["id"] != id);
            self.rows.push(json!({ "id": id, "app": "modbus-poll", "name": name, "team_id": null, "data": data, "version": version,
                "seq": self.seq, "deleted": false, "can_edit": true }));
        }
        fn tombstone(&mut self, id: &str) {
            let v = self.rows.iter().find(|r| r["id"] == id).map_or(1, |r| r["version"].as_i64().unwrap() + 1);
            self.seq += 1;
            self.rows.retain(|r| r["id"] != id);
            self.rows.push(json!({ "id": id, "app": "modbus-poll", "deleted": true, "version": v, "seq": self.seq }));
        }
    }

    fn handle(fake: &Mutex<Fake>, r: &crate::testutil::Req) -> Reply {
        let mut f = fake.lock();
        let (path, query) = r.path.split_once('?').unwrap_or((r.path.as_str(), ""));
        let q: std::collections::HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
        match (r.method.as_str(), path) {
            ("GET", "/api/connect/profiles") => {
                let since = q.get("since").and_then(|s| s.parse::<i64>().ok());
                let items: Vec<&Value> = f
                    .rows
                    .iter()
                    .filter(|row| !f.hidden.iter().any(|h| row["id"] == h.as_str()))
                    .filter(|row| match since {
                        Some(s) => row["seq"].as_i64().unwrap() > s,
                        None => row["deleted"] != true,
                    })
                    .collect();
                json(200, json!({ "items": items, "cursor": f.seq }))
            }
            ("PUT", p) => {
                let id = p.rsplit('/').next().unwrap().to_string();
                if f.refuse.contains(&id) {
                    return json(402, json!({ "error": "entitlement_required", "message": "Connect subscription required" }));
                }
                let body = r.json();
                let base = body["base_version"].as_i64().unwrap_or(0);
                let existing = f.rows.iter().find(|row| row["id"] == id.as_str()).cloned();
                match existing {
                    Some(row) if row["deleted"] == true => json(409, json!({ "error": "deleted", "message": "gone" })),
                    Some(row) if row["version"].as_i64().unwrap() != base => {
                        json(409, json!({ "error": "version_conflict", "message": "stale", "current": row }))
                    }
                    None if base != 0 => json(404, json!({ "error": "not_found", "message": "no" })),
                    other => {
                        let version = other.map_or(1, |row| row["version"].as_i64().unwrap() + 1);
                        f.put(&id, body["name"].as_str().unwrap(), body["data"].clone(), version);
                        json(200, f.rows.iter().find(|row| row["id"] == id.as_str()).unwrap().clone())
                    }
                }
            }
            ("DELETE", p) => {
                let id = p.rsplit('/').next().unwrap().to_string();
                f.tombstone(&id);
                json(200, json!({ "id": id, "deleted": true }))
            }
            _ => json(404, json!({ "error": "not_found", "message": "route" })),
        }
    }

    fn now() -> u64 {
        1_000
    }

    async fn setup() -> (Session, Arc<Mutex<Fake>>, MockServer) {
        let fake = Arc::new(Mutex::new(Fake::default()));
        let f = fake.clone();
        let server = MockServer::start(move |r| handle(&f, r)).await;
        let vault = MemoryVault::default();
        let t = TokenSet { access_token: "a".into(), refresh_token: "r".into(), expires_at: 100_000 };
        vault.set("tokens", &serde_json::to_string(&t).unwrap()).unwrap();
        let session = Session::with_clock(Api::new(&server.base).unwrap(), Box::new(vault), "modbus-poll", "voltrus-modbus-poll://connect", "t", now);
        (session, fake, server)
    }

    fn local(key: &str, data: Value, link: Option<(&str, i64)>, dirty: bool) -> LocalEntry {
        LocalEntry {
            key: key.into(),
            name: key.into(),
            data,
            team_id: None,
            link: link.map(|(id, version)| Link { id: id.into(), version }),
            dirty,
        }
    }

    fn req(locals: Vec<LocalEntry>, cursor: Option<i64>, full: bool) -> SyncRequest {
        SyncRequest { locals, deleted: vec![], cursor, full }
    }

    #[tokio::test]
    async fn first_sync_uploads_local_profiles_and_downloads_the_rest() {
        let (s, fake, _srv) = setup().await;
        fake.lock().put("11111111-1111-4111-8111-111111111111", "Boiler", json!({ "host": "10.0.0.5" }), 1);
        let res = run(&s, "modbus-poll", req(vec![local("Pump", json!({ "host": "10.0.0.9" }), None, true)], None, true)).await.unwrap();
        assert!(matches!(&res.ops[0], Op::Saved { key, profile } if key == "Pump" && profile.version == 1));
        assert!(matches!(&res.ops[1], Op::Added { profile } if profile.name == "Boiler"));
        assert_eq!(res.ops.len(), 2);
        assert!(res.cursor.is_some());
        assert_eq!(fake.lock().rows.len(), 2);
    }

    #[tokio::test]
    async fn an_edit_uploads_with_the_base_version() {
        let (s, fake, srv) = setup().await;
        let id = "22222222-2222-4222-8222-222222222222";
        fake.lock().put(id, "Pump", json!({ "n": 1 }), 3);
        let cursor = fake.lock().seq;
        let res = run(&s, "modbus-poll", req(vec![local("Pump", json!({ "n": 2 }), Some((id, 3)), true)], Some(cursor), false)).await.unwrap();
        assert!(matches!(&res.ops[0], Op::Saved { profile, .. } if profile.version == 4));
        let put = srv.requests.lock().iter().find(|r| r.method == "PUT").cloned().unwrap();
        assert_eq!(put.json()["base_version"], 3);
        assert_eq!(put.json()["app"], "modbus-poll");
    }

    #[tokio::test]
    async fn a_stale_edit_is_a_conflict_not_an_overwrite() {
        let (s, fake, _srv) = setup().await;
        let id = "33333333-3333-4333-8333-333333333333";
        fake.lock().put(id, "Pump", json!({ "n": "theirs" }), 5);
        let cursor = fake.lock().seq;
        let res = run(&s, "modbus-poll", req(vec![local("Pump", json!({ "n": "mine" }), Some((id, 4)), true)], Some(cursor), false)).await.unwrap();
        match &res.ops[..] {
            [Op::Conflict { key, server: Some(server) }] => {
                assert_eq!(key, "Pump");
                assert_eq!(server.version, 5);
                assert_eq!(server.data, json!({ "n": "theirs" }));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(fake.lock().rows[0]["data"], json!({ "n": "theirs" }), "server copy untouched");
    }

    #[tokio::test]
    async fn keeping_mine_retries_on_top_of_the_servers_version() {
        let (s, fake, _srv) = setup().await;
        let id = "44444444-4444-4444-8444-444444444444";
        fake.lock().put(id, "Pump", json!({ "n": "theirs" }), 5);
        let draft = ProfileDraft { id: id.into(), name: "Pump".into(), team_id: None, data: json!({ "n": "mine" }), base_version: 4 };
        let SaveOutcome::Conflict { server: Some(current) } = profiles::save(&s, "modbus-poll", &draft).await.unwrap() else {
            panic!("expected a conflict")
        };
        let retry = ProfileDraft { base_version: current.version, ..draft };
        let SaveOutcome::Saved { profile } = profiles::save(&s, "modbus-poll", &retry).await.unwrap() else { panic!("expected saved") };
        assert_eq!(profile.version, 6);
        assert_eq!(profile.data, json!({ "n": "mine" }));
    }

    #[tokio::test]
    async fn a_newer_server_copy_is_pulled_only_when_the_local_one_is_clean() {
        let (s, fake, _srv) = setup().await;
        let id = "55555555-5555-4555-8555-555555555555";
        fake.lock().put(id, "Pump", json!({ "n": 2 }), 2);
        let behind = run(&s, "modbus-poll", req(vec![local("Pump", json!({ "n": 1 }), Some((id, 1)), false)], Some(0), false)).await.unwrap();
        assert!(matches!(&behind.ops[..], [Op::Pulled { profile, .. }] if profile.version == 2));
        // Our own earlier save echoes back in the change feed: same version, nothing to do.
        let same = run(&s, "modbus-poll", req(vec![local("Pump", json!({ "n": 2 }), Some((id, 2)), false)], Some(0), false)).await.unwrap();
        assert!(same.ops.is_empty(), "{:?}", same.ops);
    }

    #[tokio::test]
    async fn tombstones_remove_clean_profiles_and_resurrect_edited_ones() {
        let (s, fake, _srv) = setup().await;
        let (a, b) = ("66666666-6666-4666-8666-666666666666", "77777777-7777-4777-8777-777777777777");
        {
            let mut f = fake.lock();
            f.put(a, "A", json!({}), 1);
            f.put(b, "B", json!({}), 1);
        }
        let cursor = fake.lock().seq;
        fake.lock().tombstone(a);
        fake.lock().tombstone(b);
        let res = run(
            &s,
            "modbus-poll",
            req(vec![local("A", json!({}), Some((a, 1)), false), local("B", json!({ "edited": true }), Some((b, 1)), true)], Some(cursor), false),
        )
        .await
        .unwrap();
        assert_eq!(res.ops[0], Op::Removed { key: "A".into() });
        match &res.ops[1] {
            Op::Saved { key, profile } => {
                assert_eq!(key, "B");
                assert_ne!(profile.id, b, "re-created under a new id");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn local_deletions_reach_the_server_and_do_not_come_back() {
        let (s, fake, _srv) = setup().await;
        let id = "88888888-8888-4888-8888-888888888888";
        fake.lock().put(id, "Gone", json!({}), 1);
        let res = run(&s, "modbus-poll", SyncRequest { locals: vec![], deleted: vec![id.into()], cursor: None, full: true }).await.unwrap();
        assert_eq!(res.ops, vec![Op::Deleted { id: id.into() }]);
        assert_eq!(fake.lock().rows[0]["deleted"], true);
    }

    #[tokio::test]
    async fn profiles_missing_from_a_full_list_are_not_deleted_locally() {
        // The entitlement lapsed: team profiles drop out of the list, nothing is deleted.
        let (s, fake, _srv) = setup().await;
        let id = "99999999-9999-4999-8999-999999999999";
        fake.lock().put(id, "Shared", json!({}), 1);
        fake.lock().hidden.push(id.into());
        let res = run(&s, "modbus-poll", req(vec![local("Shared", json!({}), Some((id, 1)), false)], None, true)).await.unwrap();
        assert!(res.ops.is_empty(), "{:?}", res.ops);
    }

    #[tokio::test]
    async fn one_refused_profile_does_not_stop_the_others() {
        let (s, fake, _srv) = setup().await;
        let team = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        fake.lock().put(team, "Team", json!({}), 1);
        fake.lock().refuse.push(team.into());
        let cursor = fake.lock().seq;
        let res = run(
            &s,
            "modbus-poll",
            req(vec![local("Team", json!({ "x": 1 }), Some((team, 1)), true), local("Mine", json!({}), None, true)], Some(cursor), false),
        )
        .await
        .unwrap();
        assert!(matches!(&res.ops[0], Op::Failed { key, message } if key == "Team" && message.contains("subscription")));
        assert!(matches!(&res.ops[1], Op::Saved { key, .. } if key == "Mine"));
    }
}
