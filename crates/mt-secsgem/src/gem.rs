//! GEM (SEMI E30) data types and the item layouts of the stream/function pairs this crate speaks.
//!
//! The layouts are written once here and used by both the host and the simulator, so the two
//! cannot drift apart. Message definitions follow the public descriptions in the bparzella/secsgem
//! stream/function classes and the SML in common tooling; no SEMI text is reproduced.
//!
//! Every decoder returns `Err(String)` describing the first thing that does not fit; the host
//! turns that into [`crate::SecsError::UnexpectedReply`] and the simulator into an S9F7.

use serde::{Deserialize, Serialize};

use crate::item::Item;

/// Generates an enum of a coded byte (`HCACK`, `EAC`, ...) with an `Other` variant for codes the
/// peer invents.
macro_rules! coded {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $var:ident = $code:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $($(#[$vmeta])* $var,)+
            /// A code this crate has no name for.
            Other(u8),
        }

        impl $name {
            pub fn from_code(code: u8) -> Self {
                match code {
                    $($code => Self::$var,)+
                    c => Self::Other(c),
                }
            }

            pub fn code(self) -> u8 {
                match self {
                    $(Self::$var => $code,)+
                    Self::Other(c) => c,
                }
            }

            /// Code 0, the success code of every ack in this module.
            pub fn is_ok(self) -> bool {
                self.code() == 0
            }

            /// The ack as the one-byte binary item that carries it.
            pub fn to_item(self) -> Item {
                Item::byte(self.code())
            }
        }
    };
}

coded! {
    /// `COMMACK` of S1F14.
    CommAck { Accepted = 0, Denied = 1 }
}
coded! {
    /// `OFLACK` of S1F16.
    OflAck { Acknowledged = 0 }
}
coded! {
    /// `ONLACK` of S1F18.
    OnlAck { Accepted = 0, NotAllowed = 1, AlreadyOnline = 2 }
}
coded! {
    /// `EAC` of S2F16.
    Eac {
        Accepted = 0,
        /// At least one constant does not exist.
        ConstantDoesNotExist = 1,
        Busy = 2,
        /// At least one value is out of range.
        OutOfRange = 3
    }
}
coded! {
    /// `DRACK` of S2F34.
    Drack {
        Accepted = 0,
        OutOfSpace = 1,
        InvalidFormat = 2,
        /// At least one RPTID is already defined.
        AlreadyDefined = 3,
        /// At least one VID does not exist.
        VidDoesNotExist = 4
    }
}
coded! {
    /// `LRACK` of S2F36.
    Lrack {
        Accepted = 0,
        OutOfSpace = 1,
        InvalidFormat = 2,
        /// At least one CEID already has a link.
        LinkAlreadyDefined = 3,
        CeidDoesNotExist = 4,
        RptidDoesNotExist = 5
    }
}
coded! {
    /// `ERACK` of S2F38.
    Erack { Accepted = 0, CeidDoesNotExist = 1 }
}
coded! {
    /// `HCACK` of S2F42.
    Hcack {
        Acknowledged = 0,
        CommandDoesNotExist = 1,
        CannotPerformNow = 2,
        /// At least one parameter is invalid (see the per-parameter `CPACK`).
        ParameterInvalid = 3,
        /// Accepted; completion is reported later by an event.
        AcknowledgedFinishLater = 4,
        AlreadyInCondition = 5,
        NoSuchObject = 6
    }
}
coded! {
    /// `CPACK` of S2F42.
    Cpack {
        Ok = 0,
        ParameterDoesNotExist = 1,
        IllegalValue = 2,
        IllegalFormat = 3
    }
}
coded! {
    /// `ACKC5` of S5F2 and S5F4.
    Ackc5 { Accepted = 0, Error = 1 }
}
coded! {
    /// `ACKC6` of S6F12.
    Ackc6 { Accepted = 0 }
}
coded! {
    /// `ACKC7` of S7F4.
    Ackc7 { Accepted = 0 }
}
coded! {
    /// `ACKC10` of S10F2 and S10F4.
    Ackc10 { Accepted = 0, NotDisplayed = 1, TerminalNotAvailable = 2 }
}

/// GEM control state; the number is the value of the `ControlState` status variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ControlState {
    EquipmentOffline,
    AttemptOnline,
    HostOffline,
    OnlineLocal,
    OnlineRemote,
}

impl ControlState {
    pub fn code(self) -> u8 {
        match self {
            ControlState::EquipmentOffline => 1,
            ControlState::AttemptOnline => 2,
            ControlState::HostOffline => 3,
            ControlState::OnlineLocal => 4,
            ControlState::OnlineRemote => 5,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => ControlState::EquipmentOffline,
            2 => ControlState::AttemptOnline,
            3 => ControlState::HostOffline,
            4 => ControlState::OnlineLocal,
            5 => ControlState::OnlineRemote,
            _ => return None,
        })
    }

    pub fn is_online(self) -> bool {
        matches!(self, ControlState::OnlineLocal | ControlState::OnlineRemote)
    }
}

/// GEM communication state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CommunicationState {
    /// Communication switched off at the equipment.
    Disabled,
    /// Enabled, no HSMS session or S1F13 sent and not yet answered.
    WaitCra,
    /// Enabled, the last S1F13 was refused or unanswered; waiting out the establish timeout.
    WaitDelay,
    Communicating,
}

impl CommunicationState {
    pub fn code(self) -> u8 {
        match self {
            CommunicationState::Disabled => 0,
            CommunicationState::WaitCra => 1,
            CommunicationState::WaitDelay => 2,
            CommunicationState::Communicating => 3,
        }
    }
}

/// Which numeric item type carries ids (SVID, ECID, CEID, RPTID, ALID, DATAID) in messages this
/// crate builds. Equipment differs here; the decoder accepts every integer type either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum IdFormat {
    U1,
    U2,
    #[default]
    U4,
    U8,
    I1,
    I2,
    I4,
    I8,
}

impl IdFormat {
    pub fn encode(self, id: u64) -> Item {
        match self {
            IdFormat::U1 => Item::u1(id as u8),
            IdFormat::U2 => Item::u2(id as u16),
            IdFormat::U4 => Item::u4(id as u32),
            IdFormat::U8 => Item::u8(id),
            IdFormat::I1 => Item::i1(id as i8),
            IdFormat::I2 => Item::i2(id as i16),
            IdFormat::I4 => Item::i4(id as i32),
            IdFormat::I8 => Item::i8(id as i64),
        }
    }

    /// A zero-length item of this type (the "all" selector of S5F3).
    pub fn encode_empty(self) -> Item {
        match self {
            IdFormat::U1 => Item::U1(vec![]),
            IdFormat::U2 => Item::U2(vec![]),
            IdFormat::U4 => Item::U4(vec![]),
            IdFormat::U8 => Item::U8(vec![]),
            IdFormat::I1 => Item::I1(vec![]),
            IdFormat::I2 => Item::I2(vec![]),
            IdFormat::I4 => Item::I4(vec![]),
            IdFormat::I8 => Item::I8(vec![]),
        }
    }

    pub fn list(self, ids: &[u64]) -> Item {
        Item::List(ids.iter().map(|id| self.encode(*id)).collect())
    }
}

// ---- value types -------------------------------------------------------------------------

/// `MDLN` and `SOFTREV`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub mdln: String,
    pub softrev: String,
}

/// One entry of S1F12.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SvInfo {
    pub id: u64,
    pub name: String,
    pub units: String,
}

/// One entry of S2F30. `min`, `max` and `default` are items in the constant's own format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EcInfo {
    pub id: u64,
    pub name: String,
    pub min: Item,
    pub max: Item,
    pub default: Item,
    pub units: String,
}

/// A report definition of S2F33.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportDef {
    pub rptid: u64,
    /// Variables the report carries; empty deletes the report.
    pub vids: Vec<u64>,
}

/// A link of S2F35.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLink {
    pub ceid: u64,
    /// Reports sent with the event; empty removes the link.
    pub rptids: Vec<u64>,
}

/// The values of one report in an S6F11.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportValues {
    pub rptid: u64,
    pub values: Vec<Item>,
}

/// A received S6F11.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventReport {
    pub dataid: u64,
    pub ceid: u64,
    pub reports: Vec<ReportValues>,
}

/// A received S5F1 (or one entry of S5F6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmReport {
    /// Bit 8 set: the alarm is set; bits 1-7: category.
    pub alcd: u8,
    pub alid: u64,
    pub altx: String,
}

impl AlarmReport {
    pub fn is_set(&self) -> bool {
        self.alcd & 0x80 != 0
    }

    /// Alarm category, 1 (personal safety) to 8 (data integrity); 0 when not categorised.
    pub fn category(&self) -> u8 {
        self.alcd & 0x7f
    }
}

/// A remote command parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandParam {
    pub name: String,
    pub value: Item,
}

impl CommandParam {
    pub fn new(name: impl Into<String>, value: Item) -> Self {
        CommandParam {
            name: name.into(),
            value,
        }
    }
}

/// The S2F42 answer to a remote command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandReply {
    pub hcack: Hcack,
    /// Parameters the equipment objected to, with its reason.
    pub params: Vec<(String, Cpack)>,
}

/// A received S10F1 (terminal request) or the content of an S10F3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalText {
    pub tid: u8,
    pub text: String,
}

// ---- decoding helpers --------------------------------------------------------------------

type Shape<T> = Result<T, String>;

fn describe(item: &Item) -> String {
    item.to_sml()
}

pub(crate) fn want_list<'a>(item: &'a Item, what: &str) -> Shape<&'a [Item]> {
    item.as_list()
        .ok_or_else(|| format!("{what}: expected a list, got {}", short(item)))
}

pub(crate) fn want_list_n<'a>(item: &'a Item, n: usize, what: &str) -> Shape<&'a [Item]> {
    let l = want_list(item, what)?;
    if l.len() != n {
        return Err(format!("{what}: expected a list of {n}, got {}", l.len()));
    }
    Ok(l)
}

pub(crate) fn want_id(item: &Item, what: &str) -> Shape<u64> {
    if item.len() != 1 {
        return Err(format!("{what}: expected one integer, got {}", short(item)));
    }
    item.as_u64()
        .ok_or_else(|| format!("{what}: expected an unsigned integer, got {}", short(item)))
}

pub(crate) fn want_text(item: &Item, what: &str) -> Shape<String> {
    match item {
        Item::Ascii(s) => Ok(s.clone()),
        Item::Jis8(b) => Ok(b.iter().map(|b| char::from(*b)).collect()),
        other => Err(format!("{what}: expected text, got {}", short(other))),
    }
}

pub(crate) fn want_code(item: &Item, what: &str) -> Shape<u8> {
    item.as_u64()
        .and_then(|v| u8::try_from(v).ok())
        .ok_or_else(|| format!("{what}: expected a one-byte code, got {}", short(item)))
}

pub(crate) fn want_ids(item: &Item, what: &str) -> Shape<Vec<u64>> {
    want_list(item, what)?
        .iter()
        .map(|i| want_id(i, what))
        .collect()
}

/// A short rendering for error messages.
fn short(item: &Item) -> String {
    let s = describe(item);
    if s.len() > 60 {
        format!(
            "{}...",
            &s[..s.char_indices().nth(57).map_or(s.len(), |(i, _)| i)]
        )
    } else {
        s
    }
}

// ---- S1 ----------------------------------------------------------------------------------

/// S1F2 (equipment answer) and S1F13 (equipment) body: `<L [2] <A MDLN> <A SOFTREV>>`.
pub fn encode_identity(id: &Identity) -> Item {
    Item::list([Item::ascii(&id.mdln), Item::ascii(&id.softrev)])
}

/// `<L [2] MDLN SOFTREV>`, or an empty list (host answers) as `None`.
pub fn decode_identity(item: &Item) -> Shape<Option<Identity>> {
    let l = want_list(item, "MDLN/SOFTREV")?;
    if l.is_empty() {
        return Ok(None);
    }
    let l = want_list_n(item, 2, "MDLN/SOFTREV")?;
    Ok(Some(Identity {
        mdln: want_text(&l[0], "MDLN")?,
        softrev: want_text(&l[1], "SOFTREV")?,
    }))
}

/// S1F14: `<L [2] <B COMMACK> <L [2] MDLN SOFTREV>>` (equipment) with an empty inner list from a host.
pub fn encode_s1f14(ack: CommAck, id: Option<&Identity>) -> Item {
    Item::list([
        ack.to_item(),
        id.map_or_else(Item::empty_list, encode_identity),
    ])
}

pub fn decode_s1f14(item: &Item) -> Shape<(CommAck, Option<Identity>)> {
    let l = want_list_n(item, 2, "S1F14")?;
    Ok((
        CommAck::from_code(want_code(&l[0], "COMMACK")?),
        decode_identity(&l[1])?,
    ))
}

/// S1F12 entries: `<L [3] SVID SVNAME UNITS>`.
pub fn encode_s1f12(fmt: IdFormat, svs: &[SvInfo]) -> Item {
    Item::List(
        svs.iter()
            .map(|s| {
                Item::list([
                    fmt.encode(s.id),
                    Item::ascii(&s.name),
                    Item::ascii(&s.units),
                ])
            })
            .collect(),
    )
}

pub fn decode_s1f12(item: &Item) -> Shape<Vec<SvInfo>> {
    want_list(item, "S1F12")?
        .iter()
        .map(|e| {
            let l = want_list_n(e, 3, "S1F12 entry")?;
            Ok(SvInfo {
                id: want_id(&l[0], "SVID")?,
                name: want_text(&l[1], "SVNAME")?,
                units: want_text(&l[2], "UNITS")?,
            })
        })
        .collect()
}

// ---- S2 ----------------------------------------------------------------------------------

/// S2F15 body: `<L [n] <L [2] ECID ECV>>`.
pub fn encode_s2f15(fmt: IdFormat, set: &[(u64, Item)]) -> Item {
    Item::List(
        set.iter()
            .map(|(id, v)| Item::list([fmt.encode(*id), v.clone()]))
            .collect(),
    )
}

pub fn decode_s2f15(item: &Item) -> Shape<Vec<(u64, Item)>> {
    want_list(item, "S2F15")?
        .iter()
        .map(|e| {
            let l = want_list_n(e, 2, "S2F15 entry")?;
            Ok((want_id(&l[0], "ECID")?, l[1].clone()))
        })
        .collect()
}

/// S2F30 entries: `<L [6] ECID ECNAME ECMIN ECMAX ECDEF UNITS>`.
pub fn encode_s2f30(fmt: IdFormat, ecs: &[EcInfo]) -> Item {
    Item::List(
        ecs.iter()
            .map(|e| {
                Item::list([
                    fmt.encode(e.id),
                    Item::ascii(&e.name),
                    e.min.clone(),
                    e.max.clone(),
                    e.default.clone(),
                    Item::ascii(&e.units),
                ])
            })
            .collect(),
    )
}

pub fn decode_s2f30(item: &Item) -> Shape<Vec<EcInfo>> {
    want_list(item, "S2F30")?
        .iter()
        .map(|e| {
            let l = want_list_n(e, 6, "S2F30 entry")?;
            Ok(EcInfo {
                id: want_id(&l[0], "ECID")?,
                name: want_text(&l[1], "ECNAME")?,
                min: l[2].clone(),
                max: l[3].clone(),
                default: l[4].clone(),
                units: want_text(&l[5], "UNITS")?,
            })
        })
        .collect()
}

/// S2F33 body: `<L [2] DATAID <L [n] <L [2] RPTID <L [m] VID>>>>`.
pub fn encode_s2f33(fmt: IdFormat, dataid: u64, reports: &[ReportDef]) -> Item {
    Item::list([
        fmt.encode(dataid),
        Item::List(
            reports
                .iter()
                .map(|r| Item::list([fmt.encode(r.rptid), fmt.list(&r.vids)]))
                .collect(),
        ),
    ])
}

pub fn decode_s2f33(item: &Item) -> Shape<Vec<ReportDef>> {
    let l = want_list_n(item, 2, "S2F33")?;
    want_id(&l[0], "DATAID")?;
    want_list(&l[1], "S2F33 reports")?
        .iter()
        .map(|e| {
            let r = want_list_n(e, 2, "S2F33 report")?;
            Ok(ReportDef {
                rptid: want_id(&r[0], "RPTID")?,
                vids: want_ids(&r[1], "VID list")?,
            })
        })
        .collect()
}

/// S2F35 body: `<L [2] DATAID <L [n] <L [2] CEID <L [m] RPTID>>>>`.
pub fn encode_s2f35(fmt: IdFormat, dataid: u64, links: &[EventLink]) -> Item {
    Item::list([
        fmt.encode(dataid),
        Item::List(
            links
                .iter()
                .map(|r| Item::list([fmt.encode(r.ceid), fmt.list(&r.rptids)]))
                .collect(),
        ),
    ])
}

pub fn decode_s2f35(item: &Item) -> Shape<Vec<EventLink>> {
    let l = want_list_n(item, 2, "S2F35")?;
    want_id(&l[0], "DATAID")?;
    want_list(&l[1], "S2F35 links")?
        .iter()
        .map(|e| {
            let r = want_list_n(e, 2, "S2F35 link")?;
            Ok(EventLink {
                ceid: want_id(&r[0], "CEID")?,
                rptids: want_ids(&r[1], "RPTID list")?,
            })
        })
        .collect()
}

/// S2F37 body: `<L [2] <BOOLEAN CEED> <L [n] CEID>>`.
pub fn encode_s2f37(fmt: IdFormat, enable: bool, ceids: &[u64]) -> Item {
    Item::list([Item::boolean(enable), fmt.list(ceids)])
}

pub fn decode_s2f37(item: &Item) -> Shape<(bool, Vec<u64>)> {
    let l = want_list_n(item, 2, "S2F37")?;
    let enable = l[0]
        .as_bool()
        .ok_or_else(|| format!("CEED: expected a boolean, got {}", short(&l[0])))?;
    Ok((enable, want_ids(&l[1], "CEID list")?))
}

/// S2F41 body: `<L [2] RCMD <L [n] <L [2] CPNAME CPVAL>>>`.
pub fn encode_s2f41(rcmd: &str, params: &[CommandParam]) -> Item {
    Item::list([
        Item::ascii(rcmd),
        Item::List(
            params
                .iter()
                .map(|p| Item::list([Item::ascii(&p.name), p.value.clone()]))
                .collect(),
        ),
    ])
}

pub fn decode_s2f41(item: &Item) -> Shape<(String, Vec<CommandParam>)> {
    let l = want_list_n(item, 2, "S2F41")?;
    let rcmd = want_text(&l[0], "RCMD")?;
    let params = want_list(&l[1], "S2F41 parameters")?
        .iter()
        .map(|e| {
            let p = want_list_n(e, 2, "S2F41 parameter")?;
            Ok(CommandParam {
                name: want_text(&p[0], "CPNAME")?,
                value: p[1].clone(),
            })
        })
        .collect::<Shape<_>>()?;
    Ok((rcmd, params))
}

/// S2F42 body: `<L [2] <B HCACK> <L [n] <L [2] CPNAME CPACK>>>`.
pub fn encode_s2f42(reply: &CommandReply) -> Item {
    Item::list([
        reply.hcack.to_item(),
        Item::List(
            reply
                .params
                .iter()
                .map(|(n, a)| Item::list([Item::ascii(n), a.to_item()]))
                .collect(),
        ),
    ])
}

pub fn decode_s2f42(item: &Item) -> Shape<CommandReply> {
    let l = want_list_n(item, 2, "S2F42")?;
    let params = want_list(&l[1], "S2F42 parameters")?
        .iter()
        .map(|e| {
            let p = want_list_n(e, 2, "S2F42 parameter")?;
            Ok((
                want_text(&p[0], "CPNAME")?,
                Cpack::from_code(want_code(&p[1], "CPACK")?),
            ))
        })
        .collect::<Shape<_>>()?;
    Ok(CommandReply {
        hcack: Hcack::from_code(want_code(&l[0], "HCACK")?),
        params,
    })
}

// ---- S5 ----------------------------------------------------------------------------------

/// S5F1 body: `<L [3] <B ALCD> ALID <A ALTX>>`.
pub fn encode_s5f1(fmt: IdFormat, a: &AlarmReport) -> Item {
    Item::list([Item::byte(a.alcd), fmt.encode(a.alid), Item::ascii(&a.altx)])
}

pub fn decode_s5f1(item: &Item) -> Shape<AlarmReport> {
    let l = want_list_n(item, 3, "S5F1")?;
    Ok(AlarmReport {
        alcd: want_code(&l[0], "ALCD")?,
        alid: want_id(&l[1], "ALID")?,
        altx: want_text(&l[2], "ALTX")?,
    })
}

/// S5F3 body: `<L [2] <B ALED> ALID>`; `alid` `None` selects all alarms with a zero-length id.
pub fn encode_s5f3(fmt: IdFormat, enable: bool, alid: Option<u64>) -> Item {
    Item::list([
        Item::byte(if enable { 0x80 } else { 0x00 }),
        alid.map_or_else(|| fmt.encode_empty(), |id| fmt.encode(id)),
    ])
}

pub fn decode_s5f3(item: &Item) -> Shape<(bool, Option<u64>)> {
    let l = want_list_n(item, 2, "S5F3")?;
    let aled = want_code(&l[0], "ALED")?;
    let alid = if l[1].is_empty() && l[1].is_numeric() {
        None
    } else {
        Some(want_id(&l[1], "ALID")?)
    };
    Ok((aled & 0x80 != 0, alid))
}

/// S5F6 entries: `<L [3] <B ALCD> ALID <A ALTX>>`.
pub fn encode_s5f6(fmt: IdFormat, alarms: &[AlarmReport]) -> Item {
    Item::List(alarms.iter().map(|a| encode_s5f1(fmt, a)).collect())
}

pub fn decode_s5f6(item: &Item) -> Shape<Vec<AlarmReport>> {
    want_list(item, "S5F6")?.iter().map(decode_s5f1).collect()
}

// ---- S6 ----------------------------------------------------------------------------------

/// S6F11 body: `<L [3] DATAID CEID <L [n] <L [2] RPTID <L [m] V>>>>`.
pub fn encode_s6f11(fmt: IdFormat, r: &EventReport) -> Item {
    Item::list([
        fmt.encode(r.dataid),
        fmt.encode(r.ceid),
        Item::List(
            r.reports
                .iter()
                .map(|v| Item::list([fmt.encode(v.rptid), Item::List(v.values.clone())]))
                .collect(),
        ),
    ])
}

pub fn decode_s6f11(item: &Item) -> Shape<EventReport> {
    let l = want_list_n(item, 3, "S6F11")?;
    let reports = want_list(&l[2], "S6F11 reports")?
        .iter()
        .map(|e| {
            let r = want_list_n(e, 2, "S6F11 report")?;
            Ok(ReportValues {
                rptid: want_id(&r[0], "RPTID")?,
                values: want_list(&r[1], "S6F11 values")?.to_vec(),
            })
        })
        .collect::<Shape<_>>()?;
    Ok(EventReport {
        dataid: want_id(&l[0], "DATAID")?,
        ceid: want_id(&l[1], "CEID")?,
        reports,
    })
}

// ---- S10 ---------------------------------------------------------------------------------

/// S10F1 and S10F3 body: `<L [2] <B TID> <A TEXT>>`.
pub fn encode_terminal(t: &TerminalText) -> Item {
    Item::list([Item::byte(t.tid), Item::ascii(&t.text)])
}

pub fn decode_terminal(item: &Item) -> Shape<TerminalText> {
    let l = want_list_n(item, 2, "terminal message")?;
    Ok(TerminalText {
        tid: want_code(&l[0], "TID")?,
        text: want_text(&l[1], "TEXT")?,
    })
}

/// The ack of any `<B code>` reply.
pub fn decode_ack(item: &Item, what: &str) -> Shape<u8> {
    want_code(item, what)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coded_enums() {
        assert_eq!(Hcack::from_code(4), Hcack::AcknowledgedFinishLater);
        assert_eq!(Hcack::from_code(9), Hcack::Other(9));
        assert_eq!(Hcack::Other(9).code(), 9);
        assert!(Eac::Accepted.is_ok());
        assert!(!Eac::OutOfRange.is_ok());
        assert_eq!(Eac::OutOfRange.to_item(), Item::byte(3));
        assert_eq!(OnlAck::from_code(2), OnlAck::AlreadyOnline);
    }

    #[test]
    fn s6f11_round_trip_and_sml() {
        let r = EventReport {
            dataid: 1,
            ceid: 100,
            reports: vec![ReportValues {
                rptid: 7,
                values: vec![Item::f4(1.5), Item::ascii("x")],
            }],
        };
        let item = encode_s6f11(IdFormat::U4, &r);
        assert_eq!(
            item.to_sml(),
            r#"<L [3] <U4 1> <U4 100> <L [1] <L [2] <U4 7> <L [2] <F4 1.5> <A "x">>>>>"#
        );
        assert_eq!(decode_s6f11(&item).unwrap(), r);
        // other id widths decode the same
        assert_eq!(decode_s6f11(&encode_s6f11(IdFormat::I2, &r)).unwrap(), r);
        assert!(decode_s6f11(&Item::u1(1)).is_err());
    }

    #[test]
    fn s5f3_all_alarms_is_a_zero_length_id() {
        let item = encode_s5f3(IdFormat::U4, true, None);
        assert_eq!(item.to_sml(), "<L [2] <B 0x80> <U4>>");
        assert_eq!(decode_s5f3(&item).unwrap(), (true, None));
        let item = encode_s5f3(IdFormat::U4, false, Some(5));
        assert_eq!(decode_s5f3(&item).unwrap(), (false, Some(5)));
    }

    #[test]
    fn s2f41_round_trip() {
        let params = vec![CommandParam::new("PPID", Item::ascii("A"))];
        let item = encode_s2f41("START", &params);
        assert_eq!(decode_s2f41(&item).unwrap(), ("START".to_string(), params));
        let reply = CommandReply {
            hcack: Hcack::ParameterInvalid,
            params: vec![("PPID".into(), Cpack::IllegalValue)],
        };
        assert_eq!(decode_s2f42(&encode_s2f42(&reply)).unwrap(), reply);
    }

    #[test]
    fn shape_errors_name_the_field() {
        let e = decode_s2f37(&Item::list([Item::u1(1), Item::empty_list()])).unwrap_err();
        assert!(e.contains("CEED"), "{e}");
        let e = decode_s1f14(&Item::empty_list()).unwrap_err();
        assert!(e.contains("S1F14"), "{e}");
    }
}
