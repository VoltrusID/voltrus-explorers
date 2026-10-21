//! COV subscriptions and notifications of the simulated device (ASHRAE 135
//! clauses 13.1 and 13.14, 13.6/13.7 for the notification services).
//!
//! Confirmed notifications use unique invoke ids, are retransmitted after the
//! APDU timeout and the subscription is dropped when the subscriber never
//! acknowledges them (or answers with Error / Reject / Abort).

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use bacnet_rs::object::{ObjectIdentifier, ObjectType};

use super::props::{P_PRESENT_VALUE, P_STATUS_FLAGS};
use super::wire::*;
use super::{Kind, SimObj, COV_INCREMENT};

/// APDU_Timeout / Number_Of_APDU_Retries the device advertises and applies to
/// the confirmed notifications it originates.
pub(super) const APDU_TIMEOUT: Duration = Duration::from_millis(3000);
pub(super) const APDU_RETRIES: u8 = 3;

/// ConfirmedCOVNotification (confirmed service 1) / UnconfirmedCOVNotification (unconfirmed service 2).
const CONFIRMED_COV_NOTIFICATION: u8 = 1;
const UNCONFIRMED_COV_NOTIFICATION: u8 = 2;

pub(super) struct CovSub {
    pub proc_id: u32,
    pub object: ObjectIdentifier,
    pub confirmed: bool,
    pub expires: Option<Instant>,
    pub subscriber: Peer,
    /// Value at the time of the last notification (COV change reference).
    last_value: f64,
    last_out_of_service: bool,
}

/// A confirmed notification waiting for the subscriber's acknowledgement.
struct Pending {
    invoke_id: u8,
    peer: Peer,
    proc_id: u32,
    object: ObjectIdentifier,
    frame: Vec<u8>,
    sent_at: Instant,
    retries_left: u8,
}

/// Parameters of a SubscribeCOV request. `lifetime` of `None` or 0 = indefinite.
pub(super) struct Subscription {
    pub subscriber: Peer,
    pub proc_id: u32,
    pub confirmed: bool,
    pub lifetime: Option<u32>,
}

/// Live COV subscriptions, unacknowledged confirmed notifications, and the
/// frames waiting to be sent by the device loop.
pub(super) struct CovState {
    pub subs: Vec<CovSub>,
    pending: Vec<Pending>,
    next_invoke: u8,
    pub outbox: Vec<Reply>,
}

impl CovState {
    pub(super) fn new() -> Self {
        Self { subs: Vec::new(), pending: Vec::new(), next_invoke: 0, outbox: Vec::new() }
    }

    /// Remove a subscription (identified by subscriber, process id and monitored object).
    pub(super) fn remove(&mut self, subscriber: &Peer, proc_id: u32, object: ObjectIdentifier) {
        self.subs.retain(|s| !(s.subscriber == *subscriber && s.proc_id == proc_id && s.object == object));
        self.pending.retain(|p| !(p.peer == *subscriber && p.proc_id == proc_id && p.object == object));
    }

    /// Create or renew a subscription and queue the initial notification
    /// (clause 13.14.1.3: a notification follows every successful subscription).
    /// `lifetime` of `None` or 0 means indefinite.
    pub(super) fn add(&mut self, now: Instant, device_id: u32, obj: &SimObj, sub: Subscription) {
        let Subscription { subscriber, proc_id, confirmed, lifetime } = sub;
        let object = ObjectIdentifier::new(obj.ot, obj.instance);
        self.remove(&subscriber, proc_id, object);
        self.subs.push(CovSub {
            proc_id,
            object,
            confirmed,
            expires: lifetime.filter(|&l| l > 0).map(|l| now + Duration::from_secs(u64::from(l))),
            subscriber,
            last_value: obj.current,
            last_out_of_service: obj.out_of_service,
        });
        let idx = self.subs.len() - 1;
        self.queue(idx, obj, device_id, now);
    }

    fn alloc_invoke(&mut self) -> u8 {
        for _ in 0..256 {
            let id = self.next_invoke;
            self.next_invoke = id.wrapping_add(1);
            if !self.pending.iter().any(|p| p.invoke_id == id) {
                return id;
            }
        }
        // Every id is in flight: reuse the next one rather than stall.
        let id = self.next_invoke;
        self.next_invoke = id.wrapping_add(1);
        id
    }

    /// Queue a notification of `obj`'s current state to subscription `idx`.
    fn queue(&mut self, idx: usize, obj: &SimObj, device_id: u32, now: Instant) {
        let sub = &mut self.subs[idx];
        sub.last_value = obj.current;
        sub.last_out_of_service = obj.out_of_service;
        let remaining = sub.expires.map_or(0, |t| {
            let d = t.saturating_duration_since(now);
            (d.as_secs() + u64::from(d.subsec_nanos() > 0)).clamp(1, u64::from(u32::MAX)) as u32
        });
        let (proc_id, object, confirmed, subscriber) = (sub.proc_id, sub.object, sub.confirmed, sub.subscriber.clone());
        let invoke = confirmed.then(|| self.alloc_invoke());
        let apdu = cov_notification_apdu(
            invoke,
            proc_id,
            device_id,
            object,
            remaining,
            obj.typed(obj.current),
            obj.out_of_service,
        );
        let frame = frame_to(&subscriber, &apdu, invoke.is_some(), 0);
        if let Some(invoke_id) = invoke {
            self.pending.push(Pending {
                invoke_id,
                peer: subscriber.clone(),
                proc_id,
                object,
                frame: frame.clone(),
                sent_at: now,
                retries_left: APDU_RETRIES,
            });
        }
        self.outbox.push(Reply { to: subscriber.addr, frame });
    }

    /// Drop expired subscriptions, queue notifications for every subscribed
    /// object whose value moved enough since it was last notified, and
    /// retransmit (or give up on) unacknowledged confirmed notifications.
    pub(super) fn check(&mut self, objects: &[SimObj], device_id: u32, now: Instant) {
        self.subs.retain(|s| s.expires.map_or(true, |t| t > now));
        let subs = &self.subs;
        self.pending
            .retain(|p| subs.iter().any(|s| s.subscriber == p.peer && s.proc_id == p.proc_id && s.object == p.object));

        for idx in 0..self.subs.len() {
            let s = &self.subs[idx];
            let Some(obj) = objects.iter().find(|o| o.ot == s.object.object_type && o.instance == s.object.instance)
            else {
                continue;
            };
            let moved = match obj.kind {
                Kind::Analog => (obj.current - s.last_value).abs() >= COV_INCREMENT,
                Kind::Binary | Kind::MultiState => obj.current != s.last_value,
            };
            if moved || obj.out_of_service != s.last_out_of_service {
                self.queue(idx, obj, device_id, now);
            }
        }

        let mut abandoned = Vec::new();
        for p in &mut self.pending {
            if now.saturating_duration_since(p.sent_at) < APDU_TIMEOUT {
                continue;
            }
            if p.retries_left == 0 {
                abandoned.push((p.peer.clone(), p.proc_id, p.object));
            } else {
                p.retries_left -= 1;
                p.sent_at = now;
                self.outbox.push(Reply { to: p.peer.addr, frame: p.frame.clone() });
            }
        }
        for (peer, proc_id, object) in abandoned {
            self.remove(&peer, proc_id, object);
        }
    }

    /// A SimpleAck (`success`) or Error / Reject / Abort answering one of our
    /// confirmed notifications. A failure means the subscriber cannot take
    /// notifications, so its subscription is cancelled.
    pub(super) fn on_response(&mut self, from: SocketAddr, invoke_id: u8, success: bool) {
        let Some(pos) = self.pending.iter().position(|p| p.invoke_id == invoke_id && p.peer.addr == from) else {
            return;
        };
        let p = self.pending.remove(pos);
        if !success {
            self.remove(&p.peer, p.proc_id, p.object);
        }
    }
}

/// APDU of a COV notification. `confirmed_invoke` of `Some(id)` →
/// ConfirmedCOVNotification, `None` → UnconfirmedCOVNotification.
/// `time_remaining` 0 = indefinite.
pub(super) fn cov_notification_apdu(
    confirmed_invoke: Option<u8>,
    proc_id: u32,
    device_id: u32,
    object: ObjectIdentifier,
    time_remaining: u32,
    present_value: Val,
    out_of_service: bool,
) -> Vec<u8> {
    let mut a = match confirmed_invoke {
        // ConfirmedRequest, max-APDU 1476, invoke id, service choice.
        Some(id) => vec![PDU_CONFIRMED_REQUEST << 4, 0x05, id, CONFIRMED_COV_NOTIFICATION],
        None => vec![PDU_UNCONFIRMED_REQUEST << 4, UNCONFIRMED_COV_NOTIFICATION],
    };
    ctx_unsigned(&mut a, 0, u64::from(proc_id));
    ctx_object_id(&mut a, 1, ObjectIdentifier::new(ObjectType::Device, device_id));
    ctx_object_id(&mut a, 2, object);
    ctx_unsigned(&mut a, 3, u64::from(time_remaining));
    put_open(&mut a, 4);
    ctx_unsigned(&mut a, 0, u64::from(P_PRESENT_VALUE));
    put_open(&mut a, 2);
    present_value.encode(&mut a);
    put_close(&mut a, 2);
    ctx_unsigned(&mut a, 0, u64::from(P_STATUS_FLAGS));
    put_open(&mut a, 2);
    // in-alarm, fault, overridden, out-of-service
    Val::Bits(vec![false, false, false, out_of_service]).encode(&mut a);
    put_close(&mut a, 2);
    put_close(&mut a, 4);
    a
}
