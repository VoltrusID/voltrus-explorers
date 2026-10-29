//! What the simulated equipment answers to each message a host sends.

use crate::error::S9Kind;
use crate::gem::*;
use crate::host::is_known_stream;
use crate::item::Item;
use crate::message::SecsMessage;

use super::model::{EquipmentModel, ProcessError, ProcessState, TerminalMessage};

/// The equipment's decision for one primary message.
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    /// The secondary message (`None` item: header only).
    Reply(Option<Item>),
    /// `S{n}F0`.
    Abort,
    /// A stream 9 error message about the primary message.
    S9(S9Kind),
}

fn reply(item: Item) -> Response {
    Response::Reply(Some(item))
}

impl EquipmentModel {
    /// Handle one primary message and apply its effects to the model. The caller sends the reply
    /// only when the message had the W-bit.
    pub fn handle(&mut self, msg: &SecsMessage) -> Response {
        let body = msg.item.as_ref();
        let illegal = Response::S9(S9Kind::IllegalData);
        match (msg.stream, msg.function) {
            (1, 1) => {
                if self.control_state == ControlState::EquipmentOffline {
                    return Response::Abort;
                }
                reply(encode_identity(&self.identity()))
            }
            (1, 13) => {
                if let Some(b) = body
                    && decode_identity(b).is_err()
                {
                    return illegal;
                }
                self.communication_established();
                reply(encode_s1f14(CommAck::Accepted, Some(&self.identity())))
            }
            (1, 15) => reply(self.host_request_offline().to_item()),
            (1, 17) => reply(self.host_request_online().to_item()),
            (1, 3) => match ids(body) {
                Ok(list) => reply(Item::List(
                    self.selection(&list, |m| {
                        m.status_variable_infos().iter().map(|s| s.id).collect()
                    })
                    .into_iter()
                    .map(|id| self.sv_value(id).unwrap_or_else(Item::empty_list))
                    .collect(),
                )),
                Err(_) => illegal,
            },
            (1, 11) => match ids(body) {
                Ok(list) => {
                    let infos = self.status_variable_infos();
                    let out: Vec<SvInfo> = self
                        .selection(&list, |m| {
                            m.status_variable_infos().iter().map(|s| s.id).collect()
                        })
                        .into_iter()
                        .map(|id| {
                            infos
                                .iter()
                                .find(|s| s.id == id)
                                .cloned()
                                .unwrap_or(SvInfo {
                                    id,
                                    name: String::new(),
                                    units: String::new(),
                                })
                        })
                        .collect();
                    reply(encode_s1f12(self.id_format, &out))
                }
                Err(_) => illegal,
            },
            (2, 13) => match ids(body) {
                Ok(list) => reply(Item::List(
                    self.selection(&list, |m| m.constants.iter().map(|c| c.id).collect())
                        .into_iter()
                        .map(|id| {
                            self.constant(id)
                                .map_or_else(Item::empty_list, |c| c.value.clone())
                        })
                        .collect(),
                )),
                Err(_) => illegal,
            },
            (2, 15) => match body.map(decode_s2f15) {
                Some(Ok(set)) => reply(self.set_constants(&set).to_item()),
                _ => illegal,
            },
            (2, 29) => {
                match ids(body) {
                    Ok(list) => {
                        let infos = self.constant_infos();
                        let out: Vec<EcInfo> =
                            self.selection(&list, |m| m.constants.iter().map(|c| c.id).collect())
                                .into_iter()
                                .map(|id| {
                                    infos.iter().find(|e| e.id == id).cloned().unwrap_or_else(
                                        || {
                                            let none = Item::ascii("");
                                            EcInfo {
                                                id,
                                                name: String::new(),
                                                min: none.clone(),
                                                max: none.clone(),
                                                default: none,
                                                units: String::new(),
                                            }
                                        },
                                    )
                                })
                                .collect();
                        reply(encode_s2f30(self.id_format, &out))
                    }
                    Err(_) => illegal,
                }
            }
            (2, 33) => match body.map(decode_s2f33) {
                Some(Ok(defs)) => reply(self.define_reports(&defs).to_item()),
                _ => illegal,
            },
            (2, 35) => match body.map(decode_s2f35) {
                Some(Ok(links)) => reply(self.link_reports(&links).to_item()),
                _ => illegal,
            },
            (2, 37) => match body.map(decode_s2f37) {
                Some(Ok((enable, ceids))) => reply(self.enable_events(enable, &ceids).to_item()),
                _ => illegal,
            },
            (2, 41) => match body.map(decode_s2f41) {
                Some(Ok((rcmd, params))) => {
                    reply(encode_s2f42(&self.remote_command(&rcmd, &params)))
                }
                _ => illegal,
            },
            (5, 3) => match body.map(decode_s5f3) {
                Some(Ok((enable, alid))) => reply(self.enable_alarms(enable, alid).to_item()),
                _ => illegal,
            },
            (5, 5) => match ids(body) {
                Ok(list) => {
                    let all = |m: &EquipmentModel| m.alarms.iter().map(|a| a.alid).collect();
                    let out: Vec<AlarmReport> = self
                        .selection(&list, all)
                        .into_iter()
                        .filter_map(|id| self.alarms.iter().find(|a| a.alid == id))
                        .map(|a| AlarmReport {
                            alcd: a.alcd(),
                            alid: a.alid,
                            altx: a.text.clone(),
                        })
                        .collect();
                    reply(encode_s5f6(self.id_format, &out))
                }
                Err(_) => illegal,
            },
            (7, 19) => reply(Item::List(self.ppids.iter().map(Item::ascii).collect())),
            (10, 3) => match body.map(decode_terminal) {
                Some(Ok(t)) => {
                    self.terminal_messages.push(TerminalMessage {
                        tid: t.tid,
                        text: t.text,
                    });
                    reply(Ackc10::Accepted.to_item())
                }
                _ => illegal,
            },
            (s, _) => Response::S9(if is_known_stream(s) {
                S9Kind::UnrecognizedFunction
            } else {
                S9Kind::UnrecognizedStream
            }),
        }
    }

    fn identity(&self) -> Identity {
        Identity {
            mdln: self.mdln.clone(),
            softrev: self.softrev.clone(),
        }
    }

    /// The ids a request names; an empty request means every id `all` lists.
    fn selection(&self, asked: &[u64], all: impl Fn(&EquipmentModel) -> Vec<u64>) -> Vec<u64> {
        if asked.is_empty() {
            all(self)
        } else {
            asked.to_vec()
        }
    }

    /// S2F15: nothing is applied unless every constant exists, may change now and is in range.
    fn set_constants(&mut self, set: &[(u64, Item)]) -> Eac {
        if set.iter().any(|(id, _)| self.constant(*id).is_none()) {
            return Eac::ConstantDoesNotExist;
        }
        let busy = self.chamber.state != ProcessState::Idle;
        if busy
            && set.iter().any(|(id, _)| {
                self.constant(*id)
                    .is_some_and(|c| c.locked_while_processing)
            })
        {
            return Eac::Busy;
        }
        let mut checked = Vec::with_capacity(set.len());
        for (id, value) in set {
            let c = self.constant(*id).expect("checked above");
            let number = if value.is_numeric() && value.len() == 1 {
                value.as_f64()
            } else {
                None
            };
            match number {
                Some(n)
                    if n >= c.min && n <= c.max && super::model::like(&c.value, n).is_some() =>
                {
                    checked.push((*id, n));
                }
                _ => return Eac::OutOfRange,
            }
        }
        for (id, n) in checked {
            self.set_constant(id, n);
        }
        Eac::Accepted
    }

    fn knows_vid(&self, vid: u64) -> bool {
        self.vid_value(vid).is_some()
    }

    /// S2F33. An empty definition list deletes every report (and with it every link); a report
    /// with no VIDs deletes that report.
    fn define_reports(&mut self, defs: &[ReportDef]) -> Drack {
        if defs.is_empty() {
            self.reports.clear();
            self.links.clear();
            return Drack::Accepted;
        }
        let mut seen = Vec::new();
        for d in defs {
            if seen.contains(&d.rptid) {
                return Drack::InvalidFormat;
            }
            seen.push(d.rptid);
        }
        let mut new = 0;
        for d in defs.iter().filter(|d| !d.vids.is_empty()) {
            if self.reports.iter().any(|r| r.rptid == d.rptid) {
                return Drack::AlreadyDefined;
            }
            if d.vids.iter().any(|v| !self.knows_vid(*v)) {
                return Drack::VidDoesNotExist;
            }
            new += 1;
        }
        let deleted = defs
            .iter()
            .filter(|d| d.vids.is_empty() && self.reports.iter().any(|r| r.rptid == d.rptid))
            .count();
        if self.reports.len() + new - deleted > self.max_reports {
            return Drack::OutOfSpace;
        }
        for d in defs {
            if d.vids.is_empty() {
                self.reports.retain(|r| r.rptid != d.rptid);
                for l in &mut self.links {
                    l.rptids.retain(|r| *r != d.rptid);
                }
                self.links.retain(|l| !l.rptids.is_empty());
            } else {
                self.reports.push(d.clone());
            }
        }
        Drack::Accepted
    }

    /// S2F35. An empty list removes every link; a link with no reports removes that event's link.
    fn link_reports(&mut self, links: &[EventLink]) -> Lrack {
        if links.is_empty() {
            self.links.clear();
            return Lrack::Accepted;
        }
        for l in links {
            if !self.events.iter().any(|e| e.ceid == l.ceid) {
                return Lrack::CeidDoesNotExist;
            }
        }
        for l in links {
            if l.rptids
                .iter()
                .any(|r| !self.reports.iter().any(|d| d.rptid == *r))
            {
                return Lrack::RptidDoesNotExist;
            }
        }
        for l in links {
            if !l.rptids.is_empty() && self.links.iter().any(|x| x.ceid == l.ceid) {
                return Lrack::LinkAlreadyDefined;
            }
        }
        for l in links {
            if l.rptids.is_empty() {
                self.links.retain(|x| x.ceid != l.ceid);
            } else {
                self.links.push(l.clone());
            }
        }
        Lrack::Accepted
    }

    /// S2F37. An empty list means every event.
    fn enable_events(&mut self, enable: bool, ceids: &[u64]) -> Erack {
        if ceids
            .iter()
            .any(|c| !self.events.iter().any(|e| e.ceid == *c))
        {
            return Erack::CeidDoesNotExist;
        }
        for e in &mut self.events {
            if ceids.is_empty() || ceids.contains(&e.ceid) {
                e.enabled = enable;
            }
        }
        Erack::Accepted
    }

    /// S5F3. `None` means every alarm.
    fn enable_alarms(&mut self, enable: bool, alid: Option<u64>) -> Ackc5 {
        match alid {
            None => {
                for a in &mut self.alarms {
                    a.enabled = enable;
                }
                Ackc5::Accepted
            }
            Some(id) => match self.alarms.iter_mut().find(|a| a.alid == id) {
                Some(a) => {
                    a.enabled = enable;
                    Ackc5::Accepted
                }
                None => Ackc5::Error,
            },
        }
    }

    /// S2F41: START (optional `PPID`, `LOTID`), STOP, PAUSE, RESUME. Only online remote accepts
    /// commands.
    fn remote_command(&mut self, rcmd: &str, params: &[CommandParam]) -> CommandReply {
        let name = rcmd.to_ascii_uppercase();
        let known: &[&str] = match name.as_str() {
            "START" => &["PPID", "LOTID"],
            "STOP" | "PAUSE" | "RESUME" => &[],
            _ => {
                return CommandReply {
                    hcack: Hcack::CommandDoesNotExist,
                    params: Vec::new(),
                };
            }
        };
        if self.control_state != ControlState::OnlineRemote {
            return CommandReply {
                hcack: Hcack::CannotPerformNow,
                params: Vec::new(),
            };
        }
        let mut bad = Vec::new();
        let mut ppid = None;
        let mut lot = None;
        for p in params {
            let key = p.name.to_ascii_uppercase();
            if !known.contains(&key.as_str()) {
                bad.push((p.name.clone(), Cpack::ParameterDoesNotExist));
                continue;
            }
            let Some(value) = p.value.as_str() else {
                bad.push((p.name.clone(), Cpack::IllegalFormat));
                continue;
            };
            if key == "PPID" {
                if self.ppids.iter().any(|x| x == value) {
                    ppid = Some(value.to_string());
                } else {
                    bad.push((p.name.clone(), Cpack::IllegalValue));
                }
            } else {
                lot = Some(value.to_string());
            }
        }
        if !bad.is_empty() {
            return CommandReply {
                hcack: Hcack::ParameterInvalid,
                params: bad,
            };
        }
        let state = self.chamber.state;
        let hcack = match name.as_str() {
            "START" => {
                if let Some(l) = lot {
                    self.chamber.lot_id = l;
                }
                match self.start_process(ppid.as_deref()) {
                    Ok(()) => Hcack::Acknowledged,
                    Err(ProcessError::WrongState) => Hcack::CannotPerformNow,
                    Err(ProcessError::UnknownPpid) => Hcack::ParameterInvalid,
                }
            }
            "STOP" => match self.stop_process() {
                Ok(()) => Hcack::Acknowledged,
                Err(_) => Hcack::AlreadyInCondition,
            },
            "PAUSE" => match (state, self.pause_process()) {
                (_, Ok(())) => Hcack::Acknowledged,
                (ProcessState::Paused, _) => Hcack::AlreadyInCondition,
                _ => Hcack::CannotPerformNow,
            },
            _ => match (state, self.resume_process()) {
                (_, Ok(())) => Hcack::Acknowledged,
                (ProcessState::Executing, _) => Hcack::AlreadyInCondition,
                _ => Hcack::CannotPerformNow,
            },
        };
        CommandReply {
            hcack,
            params: Vec::new(),
        }
    }
}

/// The id list of a request; a missing body is an empty list.
fn ids(body: Option<&Item>) -> Result<Vec<u64>, String> {
    match body {
        None => Ok(Vec::new()),
        Some(item) => want_ids(item, "id list"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::model::{alid, ceid, ecid};

    fn run(m: &mut EquipmentModel, sml: &str) -> Response {
        m.handle(&SecsMessage::from_sml(sml).unwrap())
    }

    fn reply_sml(r: Response) -> String {
        match r {
            Response::Reply(Some(i)) => i.to_sml(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn identity_and_state_requests() {
        let mut m = EquipmentModel::default();
        assert_eq!(
            reply_sml(run(&mut m, "S1F1 W.")),
            r#"<L [2] <A "VOLTRUS-CHAMBER"> <A "1.0.0">>"#
        );
        assert_eq!(
            reply_sml(run(&mut m, "S1F13 W <L [0]>.")),
            r#"<L [2] <B 0x00> <L [2] <A "VOLTRUS-CHAMBER"> <A "1.0.0">>>"#
        );
        assert_eq!(reply_sml(run(&mut m, "S1F17 W.")), "<B 0x02>");
        assert_eq!(reply_sml(run(&mut m, "S1F15 W.")), "<B 0x00>");
        assert_eq!(reply_sml(run(&mut m, "S1F17 W.")), "<B 0x00>");
        m.operator_offline();
        assert_eq!(run(&mut m, "S1F1 W."), Response::Abort);
        assert_eq!(reply_sml(run(&mut m, "S1F17 W.")), "<B 0x01>");
    }

    #[test]
    fn unknown_and_malformed_messages_get_s9() {
        let mut m = EquipmentModel::default();
        assert_eq!(
            run(&mut m, "S1F99 W."),
            Response::S9(S9Kind::UnrecognizedFunction)
        );
        assert_eq!(
            run(&mut m, "S99F1 W."),
            Response::S9(S9Kind::UnrecognizedStream)
        );
        assert_eq!(
            run(&mut m, "S2F37 W <U1 1>."),
            Response::S9(S9Kind::IllegalData)
        );
        assert_eq!(
            run(&mut m, "S1F3 W <A \"x\">."),
            Response::S9(S9Kind::IllegalData)
        );
    }

    #[test]
    fn status_variables() {
        let mut m = EquipmentModel::default();
        assert_eq!(
            reply_sml(run(&mut m, "S1F3 W <L [3] <U4 2> <U4 12> <U4 4242>>.")),
            "<L [3] <U1 5> <U4 0> <L [0]>>"
        );
        let all = run(&mut m, "S1F3 W <L [0]>.");
        let Response::Reply(Some(Item::List(values))) = all else {
            panic!()
        };
        assert_eq!(values.len(), 13);
        let names = reply_sml(run(&mut m, "S1F11 W <L [2] <U4 10> <U4 4242>>."));
        assert_eq!(
            names,
            r#"<L [2] <L [3] <U4 10> <A "ChamberPressure"> <A "mTorr">> <L [3] <U4 4242> <A> <A>>>"#
        );
    }

    #[test]
    fn equipment_constants() {
        let mut m = EquipmentModel::default();
        assert_eq!(
            reply_sml(run(&mut m, "S2F13 W <L [2] <U4 1002> <U4 1005>>.")),
            "<L [2] <U4 60> <U2 10>>"
        );
        assert_eq!(
            reply_sml(run(&mut m, "S2F29 W <L [1] <U4 1002>>.")),
            r#"<L [1] <L [6] <U4 1002> <A "ProcessTime"> <U4 1> <U4 3600> <U4 60> <A "s">>>"#
        );
        // accepted, converted to the constant's type
        assert_eq!(
            reply_sml(run(&mut m, "S2F15 W <L [1] <L [2] <U4 1002> <F8 90.0>>>.")),
            "<B 0x00>"
        );
        assert_eq!(m.constant(ecid::PROCESS_TIME).unwrap().value, Item::u4(90));
        // out of range, wrong type, unknown: nothing applied
        assert_eq!(
            reply_sml(run(
                &mut m,
                "S2F15 W <L [2] <L [2] <U4 1002> <U4 5>> <L [2] <U4 1003> <F4 5000.0>>>."
            )),
            "<B 0x03>"
        );
        assert_eq!(m.constant(ecid::PROCESS_TIME).unwrap().value, Item::u4(90));
        assert_eq!(
            reply_sml(run(&mut m, "S2F15 W <L [1] <L [2] <U4 1002> <A \"x\">>>.")),
            "<B 0x03>"
        );
        assert_eq!(
            reply_sml(run(&mut m, "S2F15 W <L [1] <L [2] <U4 7777> <U4 1>>>.")),
            "<B 0x01>"
        );
        // busy while processing
        m.start_process(None).unwrap();
        assert_eq!(
            reply_sml(run(&mut m, "S2F15 W <L [1] <L [2] <U4 1002> <U4 5>>>.")),
            "<B 0x02>"
        );
        assert_eq!(
            reply_sml(run(&mut m, "S2F15 W <L [1] <L [2] <U4 1004> <U2 5>>>.")),
            "<B 0x00>"
        );
    }

    #[test]
    fn reports_links_and_events() {
        let mut m = EquipmentModel::default();
        let defs = "S2F33 W <L [2] <U4 0> <L [1] <L [2] <U4 10> <L [2] <U4 12> <U4 4>>>>>.";
        assert_eq!(reply_sml(run(&mut m, defs)), "<B 0x00>");
        assert_eq!(reply_sml(run(&mut m, defs)), "<B 0x03>");
        assert_eq!(
            reply_sml(run(
                &mut m,
                "S2F33 W <L [2] <U4 0> <L [1] <L [2] <U4 11> <L [1] <U4 4242>>>>>."
            )),
            "<B 0x04>"
        );
        assert_eq!(
            reply_sml(run(
                &mut m,
                "S2F33 W <L [2] <U4 0> <L [2] <L [2] <U4 11> <L [1] <U4 1>>> <L [2] <U4 11> <L [1] <U4 1>>>>>."
            )),
            "<B 0x02>"
        );
        let link = "S2F35 W <L [2] <U4 0> <L [1] <L [2] <U4 100> <L [1] <U4 10>>>>>.";
        assert_eq!(reply_sml(run(&mut m, link)), "<B 0x00>");
        assert_eq!(reply_sml(run(&mut m, link)), "<B 0x03>");
        assert_eq!(
            reply_sml(run(
                &mut m,
                "S2F35 W <L [2] <U4 0> <L [1] <L [2] <U4 555> <L [1] <U4 10>>>>>."
            )),
            "<B 0x04>"
        );
        assert_eq!(
            reply_sml(run(
                &mut m,
                "S2F35 W <L [2] <U4 0> <L [1] <L [2] <U4 101> <L [1] <U4 99>>>>>."
            )),
            "<B 0x05>"
        );
        assert_eq!(
            reply_sml(run(
                &mut m,
                "S2F37 W <L [2] <BOOLEAN TRUE> <L [1] <U4 555>>>."
            )),
            "<B 0x01>"
        );
        assert_eq!(
            reply_sml(run(
                &mut m,
                "S2F37 W <L [2] <BOOLEAN TRUE> <L [1] <U4 100>>>."
            )),
            "<B 0x00>"
        );
        assert!(
            m.events
                .iter()
                .find(|e| e.ceid == ceid::PROCESS_STARTED)
                .unwrap()
                .enabled
        );
        assert!(
            !m.events
                .iter()
                .find(|e| e.ceid == ceid::PROCESS_COMPLETED)
                .unwrap()
                .enabled
        );
        // an empty ceid list is every event
        assert_eq!(
            reply_sml(run(&mut m, "S2F37 W <L [2] <BOOLEAN TRUE> <L [0]>>.")),
            "<B 0x00>"
        );
        assert!(m.events.iter().all(|e| e.enabled));
        assert_eq!(
            reply_sml(run(&mut m, "S2F37 W <L [2] <BOOLEAN FALSE> <L [0]>>.")),
            "<B 0x00>"
        );
        assert!(m.events.iter().all(|e| !e.enabled));
        // unlink, then delete one report, then everything
        assert_eq!(
            reply_sml(run(
                &mut m,
                "S2F35 W <L [2] <U4 0> <L [1] <L [2] <U4 100> <L [0]>>>>."
            )),
            "<B 0x00>"
        );
        assert!(m.links.is_empty());
        assert_eq!(reply_sml(run(&mut m, link)), "<B 0x00>");
        assert_eq!(
            reply_sml(run(
                &mut m,
                "S2F33 W <L [2] <U4 0> <L [1] <L [2] <U4 10> <L [0]>>>>."
            )),
            "<B 0x00>"
        );
        assert!(m.reports.is_empty() && m.links.is_empty());
        assert_eq!(reply_sml(run(&mut m, defs)), "<B 0x00>");
        assert_eq!(
            reply_sml(run(&mut m, "S2F33 W <L [2] <U4 0> <L [0]>>.")),
            "<B 0x00>"
        );
        assert!(m.reports.is_empty());
    }

    #[test]
    fn report_space_is_limited() {
        let mut m = EquipmentModel {
            max_reports: 1,
            ..Default::default()
        };
        assert_eq!(
            reply_sml(run(
                &mut m,
                "S2F33 W <L [2] <U4 0> <L [2] <L [2] <U4 1> <L [1] <U4 1>>> <L [2] <U4 2> <L [1] <U4 1>>>>>."
            )),
            "<B 0x01>"
        );
        assert!(m.reports.is_empty());
    }

    #[test]
    fn alarms() {
        let mut m = EquipmentModel::default();
        m.raise_alarm(alid::PUMP_WARNING);
        assert_eq!(
            reply_sml(run(&mut m, "S5F5 W <L [0]>.")),
            r#"<L [3] <L [3] <B 0x02> <U4 1> <A "Chamber over temperature">> <L [3] <B 0x01> <U4 2> <A "Chamber door open">> <L [3] <B 0x86> <U4 3> <A "Vacuum pump warning">>>"#
        );
        assert_eq!(
            reply_sml(run(&mut m, "S5F5 W <L [2] <U4 3> <U4 99>>.")),
            r#"<L [1] <L [3] <B 0x86> <U4 3> <A "Vacuum pump warning">>>"#
        );
        assert_eq!(
            reply_sml(run(&mut m, "S5F3 W <L [2] <B 0x00> <U4 3>>.")),
            "<B 0x00>"
        );
        assert!(!m.alarms[2].enabled);
        assert_eq!(
            reply_sml(run(&mut m, "S5F3 W <L [2] <B 0x00> <U4 99>>.")),
            "<B 0x01>"
        );
        assert_eq!(
            reply_sml(run(&mut m, "S5F3 W <L [2] <B 0x80> <U4>>.")),
            "<B 0x00>"
        );
        assert!(m.alarms.iter().all(|a| a.enabled));
    }

    #[test]
    fn remote_commands() {
        let mut m = EquipmentModel::default();
        let cmd = |m: &mut EquipmentModel, s: &str| reply_sml(run(m, s));
        assert_eq!(
            cmd(&mut m, "S2F41 W <L [2] <A \"NOPE\"> <L [0]>>."),
            "<L [2] <B 0x01> <L [0]>>"
        );
        assert_eq!(
            cmd(
                &mut m,
                "S2F41 W <L [2] <A \"START\"> <L [2] <L [2] <A \"PPID\"> <A \"BAD\">> <L [2] <A \"X\"> <U1 1>>>>."
            ),
            r#"<L [2] <B 0x03> <L [2] <L [2] <A "PPID"> <B 0x02>> <L [2] <A "X"> <B 0x01>>>>"#
        );
        assert_eq!(
            cmd(
                &mut m,
                "S2F41 W <L [2] <A \"START\"> <L [2] <L [2] <A \"PPID\"> <A \"ETCH_SIO2\">> <L [2] <A \"LOTID\"> <A \"LOT7\">>>>."
            ),
            "<L [2] <B 0x00> <L [0]>>"
        );
        assert_eq!(m.chamber.ppid, "ETCH_SIO2");
        assert_eq!(m.chamber.lot_id, "LOT7");
        assert_eq!(
            cmd(&mut m, "S2F41 W <L [2] <A \"START\"> <L [0]>>."),
            "<L [2] <B 0x02> <L [0]>>"
        );
        assert_eq!(
            cmd(&mut m, "S2F41 W <L [2] <A \"RESUME\"> <L [0]>>."),
            "<L [2] <B 0x05> <L [0]>>"
        );
        assert_eq!(
            cmd(&mut m, "S2F41 W <L [2] <A \"PAUSE\"> <L [0]>>."),
            "<L [2] <B 0x00> <L [0]>>"
        );
        assert_eq!(
            cmd(&mut m, "S2F41 W <L [2] <A \"PAUSE\"> <L [0]>>."),
            "<L [2] <B 0x05> <L [0]>>"
        );
        assert_eq!(
            cmd(&mut m, "S2F41 W <L [2] <A \"RESUME\"> <L [0]>>."),
            "<L [2] <B 0x00> <L [0]>>"
        );
        assert_eq!(
            cmd(&mut m, "S2F41 W <L [2] <A \"STOP\"> <L [0]>>."),
            "<L [2] <B 0x00> <L [0]>>"
        );
        assert_eq!(
            cmd(&mut m, "S2F41 W <L [2] <A \"STOP\"> <L [0]>>."),
            "<L [2] <B 0x05> <L [0]>>"
        );
        assert_eq!(
            cmd(&mut m, "S2F41 W <L [2] <A \"PAUSE\"> <L [0]>>."),
            "<L [2] <B 0x02> <L [0]>>"
        );
        // local control refuses remote commands
        m.operator_set_remote(false);
        assert_eq!(
            cmd(&mut m, "S2F41 W <L [2] <A \"START\"> <L [0]>>."),
            "<L [2] <B 0x02> <L [0]>>"
        );
    }

    #[test]
    fn programs_and_terminal() {
        let mut m = EquipmentModel::default();
        assert_eq!(
            reply_sml(run(&mut m, "S7F19 W.")),
            r#"<L [3] <A "WAFER_CLEAN_A"> <A "ETCH_SIO2"> <A "ANNEAL_30">>"#
        );
        assert_eq!(
            reply_sml(run(&mut m, "S10F3 W <L [2] <B 0x01> <A \"hello\">>.")),
            "<B 0x00>"
        );
        assert_eq!(
            m.terminal_messages,
            vec![TerminalMessage {
                tid: 1,
                text: "hello".into()
            }]
        );
    }
}
