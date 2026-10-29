//! GEM host <-> equipment simulator over loopback TCP: every helper of the host API, the
//! simulator's own messages, and the failure modes.

use std::time::Duration;

use mt_secsgem::sim::{alid, ceid, ecid, svid};
use mt_secsgem::*;
use tokio::sync::broadcast;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(5);

fn sim_config() -> SimulatorConfig {
    SimulatorConfig {
        mode: SimMode::Passive {
            bind_addr: "127.0.0.1".into(),
            port: 0,
        },
        initiate_communication: false,
        tick_ms: None,
        ..Default::default()
    }
}

async fn start() -> (EquipmentSimulator, GemHost) {
    start_with(sim_config(), GemHostConfig::default()).await
}

async fn start_with(
    sim: SimulatorConfig,
    mut host: GemHostConfig,
) -> (EquipmentSimulator, GemHost) {
    let sim = EquipmentSimulator::start(sim)
        .await
        .expect("simulator starts");
    host.address = format!("127.0.0.1:{}", sim.port());
    let host = GemHost::connect(host).await.expect("host connects");
    (sim, host)
}

async fn start_communicating() -> (EquipmentSimulator, GemHost) {
    let (sim, host) = start().await;
    let (ack, id) = host.establish_communications().await.unwrap();
    assert_eq!(ack, CommAck::Accepted);
    assert!(id.is_some());
    (sim, host)
}

async fn next<T: Clone>(rx: &mut broadcast::Receiver<T>) -> T {
    timeout(WAIT, rx.recv())
        .await
        .expect("event in time")
        .expect("channel open")
}

async fn next_report(rx: &mut broadcast::Receiver<HostEvent>) -> EventReport {
    loop {
        if let HostEvent::EventReport(r) = next(rx).await {
            return r;
        }
    }
}

async fn next_alarm(rx: &mut broadcast::Receiver<HostEvent>) -> AlarmReport {
    loop {
        if let HostEvent::Alarm(a) = next(rx).await {
            return a;
        }
    }
}

async fn nothing_for(rx: &mut broadcast::Receiver<HostEvent>, ms: u64) {
    if let Ok(Ok(e)) = timeout(Duration::from_millis(ms), rx.recv()).await {
        panic!("unexpected event {e:?}");
    }
}

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn establish_communications_and_are_you_there() {
    let (sim, host) = start().await;
    assert_eq!(sim.model().comm_state, CommunicationState::WaitCra);
    let id = host.are_you_there().await.unwrap().unwrap();
    assert_eq!(id.mdln, "VOLTRUS-CHAMBER");
    assert_eq!(id.softrev, "1.0.0");
    // are-you-there does not establish communication, S1F13 does
    assert_eq!(sim.model().comm_state, CommunicationState::WaitCra);

    let (ack, tool) = host.establish_communications().await.unwrap();
    assert_eq!(ack, CommAck::Accepted);
    assert_eq!(tool, Some(id));
    assert_eq!(sim.model().comm_state, CommunicationState::Communicating);
    assert!(sim.is_selected());
    assert_eq!(sim.active_sessions(), 1);
    assert_eq!(
        sim.model().sv_value(svid::COMMUNICATION_STATE),
        Some(Item::u1(3))
    );
}

#[tokio::test]
async fn offline_and_online_requests() {
    let (sim, host) = start_communicating().await;
    assert_eq!(sim.model().control_state, ControlState::OnlineRemote);
    assert_eq!(host.request_online().await.unwrap(), OnlAck::AlreadyOnline);

    assert_eq!(host.request_offline().await.unwrap(), OflAck::Acknowledged);
    assert_eq!(sim.model().control_state, ControlState::HostOffline);
    // asking again is still acknowledged
    assert_eq!(host.request_offline().await.unwrap(), OflAck::Acknowledged);
    assert!(host.are_you_there().await.unwrap().is_some());

    assert_eq!(host.request_online().await.unwrap(), OnlAck::Accepted);
    assert_eq!(sim.model().control_state, ControlState::OnlineRemote);

    // the operator took it offline: the host cannot bring it back, and S1F1 is aborted
    sim.with_model(|m| m.operator_offline());
    assert_eq!(host.request_online().await.unwrap(), OnlAck::NotAllowed);
    assert_eq!(
        host.are_you_there().await,
        Err(SecsError::Aborted { stream: 1 })
    );
    sim.with_model(|m| assert!(m.operator_online()));
    assert_eq!(sim.model().control_state, ControlState::OnlineRemote);
    assert!(host.are_you_there().await.unwrap().is_some());
}

#[tokio::test]
async fn status_variables_and_names() {
    let (sim, host) = start_communicating().await;
    let v = host
        .status_variables(&[
            svid::CONTROL_STATE,
            svid::WAFER_COUNT,
            svid::ACTIVE_PPID,
            9999,
        ])
        .await
        .unwrap();
    assert_eq!(
        v,
        vec![
            Item::u1(5),
            Item::u4(0),
            Item::ascii(""),
            Item::empty_list()
        ]
    );
    let all = host.status_variables(&[]).await.unwrap();
    assert_eq!(all.len(), 13);
    let clock = all[0].as_str().unwrap();
    assert_eq!(clock.len(), 16);

    sim.with_model(|m| {
        m.chamber.pressure_mtorr = 123.5;
        m.chamber.temperature_c = 99.0;
        m.chamber.wafer_count = 7;
    });
    let v = host
        .status_variables(&[
            svid::CHAMBER_PRESSURE,
            svid::CHAMBER_TEMPERATURE,
            svid::WAFER_COUNT,
        ])
        .await
        .unwrap();
    assert_eq!(v, vec![Item::f4(123.5), Item::f4(99.0), Item::u4(7)]);

    let names = host
        .status_variable_names(&[svid::CHAMBER_PRESSURE, 9999])
        .await
        .unwrap();
    assert_eq!(
        names,
        vec![
            SvInfo {
                id: svid::CHAMBER_PRESSURE,
                name: "ChamberPressure".into(),
                units: "mTorr".into()
            },
            SvInfo {
                id: 9999,
                name: String::new(),
                units: String::new()
            },
        ]
    );
    assert_eq!(host.status_variable_names(&[]).await.unwrap().len(), 13);
}

#[tokio::test]
async fn equipment_constants_with_range_checks() {
    let (sim, host) = start_communicating().await;
    let v = host
        .equipment_constants(&[ecid::PROCESS_TIME, ecid::PROCESS_TEMPERATURE])
        .await
        .unwrap();
    assert_eq!(v, vec![Item::u4(60), Item::f4(150.0)]);
    assert_eq!(host.equipment_constants(&[]).await.unwrap().len(), 5);

    let names = host
        .equipment_constant_names(&[ecid::PROCESS_TIME])
        .await
        .unwrap();
    assert_eq!(
        names,
        vec![EcInfo {
            id: ecid::PROCESS_TIME,
            name: "ProcessTime".into(),
            min: Item::u4(1),
            max: Item::u4(3600),
            default: Item::u4(60),
            units: "s".into()
        }]
    );
    assert_eq!(host.equipment_constant_names(&[]).await.unwrap().len(), 5);

    // set within range (a float for an integer constant converts when it is whole)
    let eac = host
        .set_equipment_constants(&[
            (ecid::PROCESS_TIME, Item::u4(120)),
            (ecid::PROCESS_TEMPERATURE, Item::f4(200.0)),
        ])
        .await
        .unwrap();
    assert_eq!(eac, Eac::Accepted);
    assert_eq!(
        host.equipment_constants(&[ecid::PROCESS_TIME, ecid::PROCESS_TEMPERATURE])
            .await
            .unwrap(),
        vec![Item::u4(120), Item::f4(200.0)]
    );
    assert_eq!(
        host.set_equipment_constants(&[(ecid::PROCESS_TIME, Item::f8(30.0))])
            .await
            .unwrap(),
        Eac::Accepted
    );
    assert_eq!(
        sim.model().constant(ecid::PROCESS_TIME).unwrap().value,
        Item::u4(30)
    );

    // out of range, fractional for an integer constant, wrong class: nothing changes
    for bad in [
        Item::u4(0),
        Item::u4(3601),
        Item::f4(1.5),
        Item::ascii("x"),
        Item::i4(-1),
    ] {
        let eac = host
            .set_equipment_constants(&[
                (ecid::MAX_WAFERS_PER_LOT, Item::u2(10)),
                (ecid::PROCESS_TIME, bad.clone()),
            ])
            .await
            .unwrap();
        assert_eq!(eac, Eac::OutOfRange, "{bad:?}");
    }
    assert_eq!(
        sim.model()
            .constant(ecid::MAX_WAFERS_PER_LOT)
            .unwrap()
            .value,
        Item::u2(25)
    );
    assert_eq!(
        host.set_equipment_constants(&[(4242, Item::u1(1))])
            .await
            .unwrap(),
        Eac::ConstantDoesNotExist
    );

    // busy while a process runs, but constants that are not locked still change
    sim.with_model(|m| m.start_process(None).unwrap());
    assert_eq!(
        host.set_equipment_constants(&[(ecid::PROCESS_TIME, Item::u4(10))])
            .await
            .unwrap(),
        Eac::Busy
    );
    assert_eq!(
        host.set_equipment_constants(&[(ecid::MAX_WAFERS_PER_LOT, Item::u2(10))])
            .await
            .unwrap(),
        Eac::Accepted
    );
}

#[tokio::test]
async fn define_link_enable_then_receive_event_reports() {
    let (sim, host) = start_communicating().await;
    let mut events = host.subscribe();
    let mut log = sim.subscribe_log();

    let drack = host
        .define_reports(&[ReportDef {
            rptid: 1,
            vids: vec![
                svid::WAFER_COUNT,
                svid::PROCESS_STATE,
                ecid::PROCESS_TIME,
                svid::ACTIVE_PPID,
            ],
        }])
        .await
        .unwrap();
    assert_eq!(drack, Drack::Accepted);
    // duplicates and unknown VIDs are refused
    assert_eq!(
        host.define_reports(&[ReportDef {
            rptid: 1,
            vids: vec![svid::WAFER_COUNT]
        }])
        .await
        .unwrap(),
        Drack::AlreadyDefined
    );
    assert_eq!(
        host.define_reports(&[ReportDef {
            rptid: 2,
            vids: vec![123_456]
        }])
        .await
        .unwrap(),
        Drack::VidDoesNotExist
    );

    let lrack = host
        .link_event_reports(&[
            EventLink {
                ceid: ceid::PROCESS_STARTED,
                rptids: vec![1],
            },
            EventLink {
                ceid: ceid::PROCESS_COMPLETED,
                rptids: vec![1],
            },
        ])
        .await
        .unwrap();
    assert_eq!(lrack, Lrack::Accepted);
    assert_eq!(
        host.link_event_reports(&[EventLink {
            ceid: 777,
            rptids: vec![1]
        }])
        .await
        .unwrap(),
        Lrack::CeidDoesNotExist
    );
    assert_eq!(
        host.link_event_reports(&[EventLink {
            ceid: ceid::PROCESS_PAUSED,
            rptids: vec![9]
        }])
        .await
        .unwrap(),
        Lrack::RptidDoesNotExist
    );
    assert_eq!(
        host.link_event_reports(&[EventLink {
            ceid: ceid::PROCESS_STARTED,
            rptids: vec![1]
        }])
        .await
        .unwrap(),
        Lrack::LinkAlreadyDefined
    );

    // events are disabled until the host enables them
    let reply = host
        .remote_command(
            "START",
            &[CommandParam::new("PPID", Item::ascii("ETCH_SIO2"))],
        )
        .await
        .unwrap();
    assert_eq!(reply.hcack, Hcack::Acknowledged);
    nothing_for(&mut events, 150).await;
    host.remote_command("STOP", &[]).await.unwrap();

    assert_eq!(
        host.enable_events(true, &[ceid::PROCESS_STARTED, ceid::PROCESS_COMPLETED])
            .await
            .unwrap(),
        Erack::Accepted
    );
    assert_eq!(
        host.enable_events(true, &[31337]).await.unwrap(),
        Erack::CeidDoesNotExist
    );

    sim.with_model(|m| m.set_constant(ecid::PROCESS_TIME, 2.0));
    host.remote_command(
        "START",
        &[CommandParam::new("PPID", Item::ascii("ETCH_SIO2"))],
    )
    .await
    .unwrap();
    let r = next_report(&mut events).await;
    assert_eq!(r.ceid, ceid::PROCESS_STARTED);
    assert_eq!(r.reports.len(), 1);
    assert_eq!(r.reports[0].rptid, 1);
    assert_eq!(
        r.reports[0].values,
        vec![
            Item::u4(0),
            Item::u1(1),
            Item::u4(2),
            Item::ascii("ETCH_SIO2")
        ]
    );

    // finishing the process fires the completed event with the new wafer count
    sim.advance(Duration::from_secs(3));
    let r = next_report(&mut events).await;
    assert_eq!(r.ceid, ceid::PROCESS_COMPLETED);
    assert_eq!(r.reports[0].values[0], Item::u4(1));
    assert_eq!(r.reports[0].values[1], Item::u1(0));
    assert!(r.dataid >= 2);

    // the host acknowledged with S6F12 (the simulator log shows it)
    let mut acks = 0;
    while acks < 2 {
        let e = next(&mut log).await;
        if e.direction == Direction::Rx && e.kind == "S6F12" {
            acks += 1;
            assert!(e.text.contains("<B 0x00>"), "{}", e.text);
        }
    }

    // disable one event: it stops arriving
    assert_eq!(
        host.enable_events(false, &[ceid::PROCESS_STARTED])
            .await
            .unwrap(),
        Erack::Accepted
    );
    host.remote_command("START", &[]).await.unwrap();
    nothing_for(&mut events, 150).await;
    host.remote_command("STOP", &[]).await.unwrap();

    // unlink and delete everything
    assert_eq!(
        host.link_event_reports(&[EventLink {
            ceid: ceid::PROCESS_COMPLETED,
            rptids: vec![]
        }])
        .await
        .unwrap(),
        Lrack::Accepted
    );
    assert_eq!(host.define_reports(&[]).await.unwrap(), Drack::Accepted);
    assert!(sim.model().reports.is_empty());
    assert!(sim.model().links.is_empty());
}

#[tokio::test]
async fn event_without_reports_and_control_state_events() {
    let (sim, host) = start_communicating().await;
    let mut events = host.subscribe();
    assert_eq!(
        host.enable_events(true, &[]).await.unwrap(),
        Erack::Accepted
    );

    // an enabled event with no report linked still arrives, with an empty report list
    sim.with_model(|m| m.start_process(None).unwrap());
    let r = next_report(&mut events).await;
    assert_eq!(r.ceid, ceid::PROCESS_STARTED);
    assert!(r.reports.is_empty());
    sim.with_model(|m| m.stop_process().unwrap());
    assert_eq!(next_report(&mut events).await.ceid, ceid::PROCESS_STOPPED);

    // control state changes are reported, including the one that takes the equipment offline
    host.request_offline().await.unwrap();
    assert_eq!(next_report(&mut events).await.ceid, ceid::HOST_OFFLINE);
    host.request_online().await.unwrap();
    assert_eq!(next_report(&mut events).await.ceid, ceid::ONLINE_REMOTE);
    sim.with_model(|m| m.operator_set_remote(false));
    assert_eq!(next_report(&mut events).await.ceid, ceid::ONLINE_LOCAL);

    // while host offline other events are not sent
    host.request_offline().await.unwrap();
    assert_eq!(next_report(&mut events).await.ceid, ceid::HOST_OFFLINE);
    sim.with_model(|m| m.start_process(None).unwrap());
    nothing_for(&mut events, 150).await;
}

#[tokio::test]
async fn alarms_are_reported_listed_and_enabled() {
    let (sim, host) = start_communicating().await;
    let mut events = host.subscribe();
    let mut log = sim.subscribe_log();

    sim.with_model(|m| m.raise_alarm(alid::DOOR_OPEN));
    let a = next_alarm(&mut events).await;
    assert_eq!(
        a,
        AlarmReport {
            alcd: 0x81,
            alid: alid::DOOR_OPEN,
            altx: "Chamber door open".into()
        }
    );
    assert!(a.is_set());
    assert_eq!(a.category(), 1);

    let listed = host.list_alarms(&[]).await.unwrap();
    assert_eq!(listed.len(), 3);
    assert!(
        listed
            .iter()
            .find(|x| x.alid == alid::DOOR_OPEN)
            .unwrap()
            .is_set()
    );
    assert!(
        !listed
            .iter()
            .find(|x| x.alid == alid::OVER_TEMPERATURE)
            .unwrap()
            .is_set()
    );
    assert_eq!(
        host.list_alarms(&[alid::PUMP_WARNING, 99])
            .await
            .unwrap()
            .len(),
        1
    );

    sim.with_model(|m| m.clear_alarm(alid::DOOR_OPEN));
    let a = next_alarm(&mut events).await;
    assert!(!a.is_set());
    assert_eq!(a.alcd, 0x01);

    // S5F2 acknowledgements went back
    let mut acks = 0;
    while acks < 2 {
        let e = next(&mut log).await;
        if e.direction == Direction::Rx && e.kind == "S5F2" {
            acks += 1;
        }
    }

    // disabled alarms still change state but are not reported
    assert_eq!(
        host.enable_alarms(false, Some(alid::OVER_TEMPERATURE))
            .await
            .unwrap(),
        Ackc5::Accepted
    );
    sim.with_model(|m| m.raise_alarm(alid::OVER_TEMPERATURE));
    nothing_for(&mut events, 150).await;
    assert!(host.list_alarms(&[alid::OVER_TEMPERATURE]).await.unwrap()[0].is_set());
    assert_eq!(
        host.enable_alarms(false, Some(4242)).await.unwrap(),
        Ackc5::Error
    );

    // all alarms on again
    assert_eq!(
        host.enable_alarms(true, None).await.unwrap(),
        Ackc5::Accepted
    );
    sim.with_model(|m| m.raise_alarm(alid::PUMP_WARNING));
    assert_eq!(next_alarm(&mut events).await.alid, alid::PUMP_WARNING);

    // the alarm-set event carries the AlarmsSet variable when linked
    host.define_reports(&[ReportDef {
        rptid: 9,
        vids: vec![svid::ALARMS_SET],
    }])
    .await
    .unwrap();
    host.link_event_reports(&[EventLink {
        ceid: ceid::ALARM_CLEARED,
        rptids: vec![9],
    }])
    .await
    .unwrap();
    host.enable_events(true, &[ceid::ALARM_CLEARED])
        .await
        .unwrap();
    sim.with_model(|m| m.clear_alarm(alid::PUMP_WARNING));
    let r = next_report(&mut events).await;
    assert_eq!(r.ceid, ceid::ALARM_CLEARED);
    assert_eq!(
        r.reports[0].values,
        vec![Item::U4(vec![alid::OVER_TEMPERATURE as u32])]
    );
}

#[tokio::test]
async fn remote_commands_drive_the_process() {
    let (sim, host) = start_communicating().await;
    let cmd = |name: &'static str, params: Vec<CommandParam>| {
        let host = &host;
        async move { host.remote_command(name, &params).await.unwrap() }
    };

    assert_eq!(cmd("BOGUS", vec![]).await.hcack, Hcack::CommandDoesNotExist);
    let bad = cmd(
        "START",
        vec![
            CommandParam::new("PPID", Item::ascii("NOPE")),
            CommandParam::new("X", Item::u1(1)),
        ],
    )
    .await;
    assert_eq!(bad.hcack, Hcack::ParameterInvalid);
    assert_eq!(
        bad.params,
        vec![
            ("PPID".to_string(), Cpack::IllegalValue),
            ("X".to_string(), Cpack::ParameterDoesNotExist)
        ]
    );
    assert_eq!(sim.model().chamber.state, ProcessState::Idle);

    assert_eq!(cmd("PAUSE", vec![]).await.hcack, Hcack::CannotPerformNow);
    let ok = cmd(
        "START",
        vec![
            CommandParam::new("PPID", Item::ascii("ANNEAL_30")),
            CommandParam::new("LOTID", Item::ascii("LOT42")),
        ],
    )
    .await;
    assert_eq!(ok.hcack, Hcack::Acknowledged);
    assert_eq!(sim.model().chamber.state, ProcessState::Executing);
    assert_eq!(sim.model().chamber.ppid, "ANNEAL_30");
    assert_eq!(sim.model().chamber.lot_id, "LOT42");
    assert_eq!(
        host.status_variables(&[svid::PROCESS_STATE, svid::LOT_ID])
            .await
            .unwrap(),
        vec![Item::u1(1), Item::ascii("LOT42")]
    );

    assert_eq!(cmd("START", vec![]).await.hcack, Hcack::CannotPerformNow);
    assert_eq!(cmd("RESUME", vec![]).await.hcack, Hcack::AlreadyInCondition);
    assert_eq!(cmd("PAUSE", vec![]).await.hcack, Hcack::Acknowledged);
    assert_eq!(sim.model().chamber.state, ProcessState::Paused);
    assert_eq!(cmd("PAUSE", vec![]).await.hcack, Hcack::AlreadyInCondition);
    assert_eq!(cmd("RESUME", vec![]).await.hcack, Hcack::Acknowledged);
    assert_eq!(sim.model().chamber.state, ProcessState::Executing);
    assert_eq!(cmd("STOP", vec![]).await.hcack, Hcack::Acknowledged);
    assert_eq!(sim.model().chamber.state, ProcessState::Idle);
    assert_eq!(cmd("STOP", vec![]).await.hcack, Hcack::AlreadyInCondition);

    // the operator switches to local: remote commands are refused
    sim.with_model(|m| m.operator_set_remote(false));
    assert_eq!(cmd("START", vec![]).await.hcack, Hcack::CannotPerformNow);
    assert_eq!(sim.model().chamber.state, ProcessState::Idle);
}

#[tokio::test]
async fn process_programs_and_terminal_display() {
    let (sim, host) = start_communicating().await;
    assert_eq!(
        host.process_program_list().await.unwrap(),
        vec!["WAFER_CLEAN_A", "ETCH_SIO2", "ANNEAL_30"]
    );
    sim.with_model(|m| m.ppids.push("NEW_RECIPE".into()));
    assert_eq!(host.process_program_list().await.unwrap().len(), 4);

    assert_eq!(
        host.terminal_display(1, "Hello operator").await.unwrap(),
        Ackc10::Accepted
    );
    assert_eq!(
        host.terminal_display(2, "Second line").await.unwrap(),
        Ackc10::Accepted
    );
    assert_eq!(
        sim.model().terminal_messages,
        vec![
            TerminalMessage {
                tid: 1,
                text: "Hello operator".into()
            },
            TerminalMessage {
                tid: 2,
                text: "Second line".into()
            },
        ]
    );
}

#[tokio::test]
async fn raw_messages_and_sml() {
    let (_sim, host) = start_communicating().await;
    let r = host.send_sml("S1F1 W.").await.unwrap();
    assert_eq!(
        r.to_sml(),
        r#"S1F2 <L [2] <A "VOLTRUS-CHAMBER"> <A "1.0.0">>."#
    );
    let r = host.send_sml("S1F3 W <L [1] <U4 12>>.").await.unwrap();
    assert_eq!(r.item, Some(Item::list([Item::u4(0)])));
    // ids of another width are understood too
    let r = host.send_sml("S1F3 W <L [1] <U1 12>>.").await.unwrap();
    assert_eq!(r.item, Some(Item::list([Item::u4(0)])));
    let r = host
        .send_raw(SecsMessage::request(7, 19, None))
        .await
        .unwrap();
    assert_eq!(r.item.unwrap().len(), 3);
    assert!(matches!(
        host.send_sml("not sml").await,
        Err(SecsError::Codec(_))
    ));
}

#[tokio::test]
async fn s9_errors_for_unknown_and_malformed_messages() {
    let (sim, host) = start_communicating().await;

    for (sml, kind) in [
        ("S1F99 W.", S9Kind::UnrecognizedFunction),
        ("S2F99 W.", S9Kind::UnrecognizedFunction),
        ("S100F1 W.", S9Kind::UnrecognizedStream),
        ("S2F37 W <U1 1>.", S9Kind::IllegalData),
        ("S1F3 W <A \"x\">.", S9Kind::IllegalData),
        ("S6F11 W <L [0]>.", S9Kind::UnrecognizedFunction),
    ] {
        match host.send_sml(sml).await {
            Err(SecsError::S9(e)) => {
                assert_eq!(e.kind, kind, "{sml}");
                assert_eq!(e.stream(), SecsMessage::from_sml(sml).unwrap().stream);
            }
            other => panic!("{sml}: {other:?}"),
        }
    }
    // the connection survives
    assert!(host.are_you_there().await.unwrap().is_some());
    assert_eq!(sim.active_sessions(), 1);

    // a malformed S6F11 text (bad item) comes back as S9F7 too
    let err = host
        .send_raw(SecsMessage::request(
            2,
            13,
            Some(Item::list([Item::ascii("not an id")])),
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            SecsError::S9(S9Error {
                kind: S9Kind::IllegalData,
                ..
            })
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn wrong_device_id_gets_s9f1() {
    let mut cfg = GemHostConfig::default();
    cfg.hsms.device_id = 5;
    let (_sim, host) = start_with(sim_config(), cfg).await;
    match host.are_you_there().await {
        Err(SecsError::S9(e)) => assert_eq!(e.kind, S9Kind::UnrecognizedDeviceId),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn equipment_initiated_communication_and_retry() {
    // the simulator sends S1F13 itself; the host answers automatically
    let mut cfg = sim_config();
    cfg.initiate_communication = true;
    let (sim, _host) = start_with(cfg, GemHostConfig::default()).await;
    wait_until("communicating", || {
        sim.model().comm_state == CommunicationState::Communicating
    })
    .await;

    // a host that refuses first: wait-delay, then another S1F13 after the establish timeout
    let mut cfg = sim_config();
    cfg.initiate_communication = true;
    let sim = EquipmentSimulator::start(cfg).await.unwrap();
    sim.with_model(|m| {
        m.set_constant(ecid::ESTABLISH_COMM_TIMEOUT, 1.0);
    });
    let (conn, mut rx) = HsmsConnection::connect(("127.0.0.1", sim.port()), HsmsConfig::default())
        .await
        .unwrap();
    let first = timeout(WAIT, rx.recv()).await.unwrap().unwrap();
    assert_eq!((first.stream(), first.function()), (1, 13));
    assert_eq!(
        first
            .message
            .as_ref()
            .unwrap()
            .item
            .as_ref()
            .unwrap()
            .to_sml(),
        r#"<L [2] <A "VOLTRUS-CHAMBER"> <A "1.0.0">>"#
    );
    conn.reply(
        &first,
        first
            .message
            .as_ref()
            .unwrap()
            .reply(Some(encode_s1f14(CommAck::Denied, None))),
    )
    .unwrap();
    wait_until("wait delay", || {
        sim.model().comm_state == CommunicationState::WaitDelay
    })
    .await;

    let start = std::time::Instant::now();
    let second = timeout(Duration::from_secs(4), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((second.stream(), second.function()), (1, 13));
    assert!(
        start.elapsed() >= Duration::from_millis(800),
        "{:?}",
        start.elapsed()
    );
    conn.reply(
        &second,
        second
            .message
            .as_ref()
            .unwrap()
            .reply(Some(encode_s1f14(CommAck::Accepted, None))),
    )
    .unwrap();
    wait_until("communicating", || {
        sim.model().comm_state == CommunicationState::Communicating
    })
    .await;
}

#[tokio::test]
async fn host_disconnect_resets_the_equipment_session() {
    let (sim, host) = start_communicating().await;
    assert_eq!(sim.active_sessions(), 1);
    host.disconnect();
    wait_until("session end", || sim.active_sessions() == 0).await;
    assert_eq!(sim.model().comm_state, CommunicationState::WaitCra);
    assert!(!sim.is_selected());
    assert!(matches!(
        host.are_you_there().await,
        Err(SecsError::NotConnected)
    ));

    // and a new host can connect again
    let again = GemHost::connect(GemHostConfig::new(format!("127.0.0.1:{}", sim.port())))
        .await
        .unwrap();
    assert!(again.are_you_there().await.unwrap().is_some());
}

#[tokio::test]
async fn a_second_host_is_refused_while_one_is_selected() {
    let (sim, _host) = start_communicating().await;
    let second = GemHost::connect(GemHostConfig::new(format!("127.0.0.1:{}", sim.port()))).await;
    assert!(second.is_err(), "{:?}", second.is_ok());
    assert_eq!(sim.active_sessions(), 1);
}

#[tokio::test]
async fn simulator_ticks_in_real_time_and_logs_messages() {
    let mut cfg = sim_config();
    cfg.tick_ms = Some(20);
    let sim = EquipmentSimulator::start(cfg).await.unwrap();
    let mut log = sim.subscribe_log();
    let host = GemHost::connect(GemHostConfig::new(format!("127.0.0.1:{}", sim.port())))
        .await
        .unwrap();
    host.establish_communications().await.unwrap();
    host.enable_events(true, &[ceid::PROCESS_COMPLETED])
        .await
        .unwrap();
    let mut events = host.subscribe();
    sim.with_model(|m| {
        m.set_constant(ecid::PROCESS_TIME, 1.0);
        m.start_process(None).unwrap();
    });
    let r = timeout(Duration::from_secs(4), next_report(&mut events))
        .await
        .unwrap();
    assert_eq!(r.ceid, ceid::PROCESS_COMPLETED);
    assert_eq!(sim.model().chamber.wafer_count, 1);

    let mut lines = Vec::new();
    while let Ok(e) = log.try_recv() {
        lines.push(e.line());
    }
    let all = lines.join("\n");
    assert!(all.contains("<- ") && all.contains("S1F13 W"), "{all}");
    assert!(all.contains("-> ") && all.contains("S1F14"), "{all}");
    assert!(all.contains("Select.req"), "{all}");
    assert!(all.contains("S6F11 W"), "{all}");
    assert!(sim.requests_served() >= 2);
}

#[tokio::test]
async fn model_survives_a_json_round_trip_while_running() {
    let (sim, host) = start_communicating().await;
    let mut saved = sim.model();
    saved.mdln = "OTHER-TOOL".into();
    saved.softrev = "9.9".into();
    saved.chamber.wafer_count = 41;
    let json = serde_json::to_string(&saved).unwrap();
    let loaded: EquipmentModel = serde_json::from_str(&json).unwrap();
    sim.with_model(|m| {
        // keep the live communication and control state, take everything else
        let (comm, control) = (m.comm_state, m.control_state);
        *m = loaded;
        m.comm_state = comm;
        m.control_state = control;
    });
    let id = host.are_you_there().await.unwrap().unwrap();
    assert_eq!(id.mdln, "OTHER-TOOL");
    assert_eq!(
        host.status_variables(&[svid::WAFER_COUNT]).await.unwrap(),
        vec![Item::u4(41)]
    );
}

#[tokio::test]
async fn passive_host_with_active_simulator() {
    let listener = HsmsListener::bind(("127.0.0.1", 0), HsmsConfig::default())
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut cfg = SimulatorConfig::active(format!("127.0.0.1:{port}"));
    cfg.initiate_communication = false;
    cfg.tick_ms = None;
    cfg.hsms.t5_ms = 300;
    let sim = EquipmentSimulator::start(cfg).await.unwrap();
    assert_eq!(sim.port(), 0);

    let host = timeout(WAIT, GemHost::accept(&listener, IdFormat::U4))
        .await
        .unwrap()
        .unwrap();
    let (ack, _) = host.establish_communications().await.unwrap();
    assert_eq!(ack, CommAck::Accepted);
    assert_eq!(
        host.status_variables(&[svid::CONTROL_STATE]).await.unwrap(),
        vec![Item::u1(5)]
    );

    // the host drops the session; the simulator reconnects (after T5)
    let mut events = host.subscribe();
    host.disconnect();
    loop {
        if let HostEvent::Closed { .. } = next(&mut events).await {
            break;
        }
    }
    let start = std::time::Instant::now();
    let host2 = timeout(WAIT, GemHost::accept(&listener, IdFormat::U4))
        .await
        .unwrap()
        .unwrap();
    assert!(
        start.elapsed() >= Duration::from_millis(150),
        "{:?}",
        start.elapsed()
    );
    assert!(host2.are_you_there().await.unwrap().is_some());
    sim.stop().await;
}

#[tokio::test]
async fn stop_closes_the_session() {
    let (sim, host) = start_communicating().await;
    let mut events = host.subscribe();
    sim.stop().await;
    loop {
        if let HostEvent::Closed { .. } = next(&mut events).await {
            break;
        }
    }
    assert!(host.are_you_there().await.is_err());
}

/// The host side of things the equipment sends by itself, against a hand-driven equipment.
#[tokio::test]
async fn host_answers_what_the_equipment_sends() {
    let listener = HsmsListener::bind(("127.0.0.1", 0), HsmsConfig::default())
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let accept =
        tokio::spawn(async move { GemHost::accept(&listener, IdFormat::U4).await.unwrap() });
    let (equipment, _erx) = HsmsConnection::connect(("127.0.0.1", port), HsmsConfig::default())
        .await
        .unwrap();
    let host = accept.await.unwrap();
    let mut events = host.subscribe();

    // S6F11 -> event + S6F12 <B 0>
    let s6f11 = SecsMessage::from_sml(
        "S6F11 W <L [3] <U4 1> <U4 5> <L [1] <L [2] <U4 2> <L [1] <A \"v\">>>>>.",
    )
    .unwrap();
    let r = equipment.request(s6f11).await.unwrap();
    assert_eq!(r.to_sml(), "S6F12 <B 0x00>.");
    assert_eq!(
        next_report(&mut events).await,
        EventReport {
            dataid: 1,
            ceid: 5,
            reports: vec![ReportValues {
                rptid: 2,
                values: vec![Item::ascii("v")]
            }]
        }
    );

    // S5F1 -> alarm + S5F2
    let r = equipment
        .request(SecsMessage::from_sml("S5F1 W <L [3] <B 0x82> <U4 7> <A \"hot\">>.").unwrap())
        .await
        .unwrap();
    assert_eq!(r.to_sml(), "S5F2 <B 0x00>.");
    assert_eq!(
        next_alarm(&mut events).await,
        AlarmReport {
            alcd: 0x82,
            alid: 7,
            altx: "hot".into()
        }
    );

    // S10F1 -> terminal + S10F2
    let r = equipment
        .request(SecsMessage::from_sml("S10F1 W <L [2] <B 0x00> <A \"msg\">>.").unwrap())
        .await
        .unwrap();
    assert_eq!(r.to_sml(), "S10F2 <B 0x00>.");
    loop {
        if let HostEvent::Terminal(t) = next(&mut events).await {
            assert_eq!(
                t,
                TerminalText {
                    tid: 0,
                    text: "msg".into()
                }
            );
            break;
        }
    }

    // S1F1 and S1F13 from the equipment
    let r = equipment
        .request(SecsMessage::request(1, 1, None))
        .await
        .unwrap();
    assert_eq!(r.to_sml(), "S1F2 <L [0]>.");
    let r = equipment
        .request(SecsMessage::from_sml("S1F13 W <L [2] <A \"M\"> <A \"1\">>.").unwrap())
        .await
        .unwrap();
    assert_eq!(r.to_sml(), "S1F14 <L [2] <B 0x00> <L [0]>>.");

    // bad S6F11 data -> S9F7; unknown function -> S9F5 + Unhandled; unknown stream -> S9F3
    for (sml, kind) in [
        ("S6F11 W <L [1] <U1 1>>.", S9Kind::IllegalData),
        ("S3F1 W.", S9Kind::UnrecognizedFunction),
        ("S100F1 W.", S9Kind::UnrecognizedStream),
    ] {
        match equipment.request(SecsMessage::from_sml(sml).unwrap()).await {
            Err(SecsError::S9(e)) => assert_eq!(e.kind, kind, "{sml}"),
            other => panic!("{sml}: {other:?}"),
        }
    }
    loop {
        if let HostEvent::Unhandled(m) = next(&mut events).await {
            assert_eq!(m.name(), "S3F1");
            break;
        }
    }

    // S9 messages the equipment sends show up as events
    let mhead = Header {
        session_id: 0,
        byte2: 0x81,
        byte3: 1,
        ptype: 0,
        stype: 0,
        system: 77,
    };
    equipment
        .send_s9(S9Kind::TransactionTimerTimeout, &mhead)
        .unwrap();
    loop {
        if let HostEvent::S9(e) = next(&mut events).await {
            assert_eq!(e.kind, S9Kind::TransactionTimerTimeout);
            assert_eq!(e.system_bytes(), 77);
            break;
        }
    }
}

/// What the host reports when the equipment answers with something that is not the documented
/// shape, and when a reply does not arrive.
#[tokio::test]
async fn host_reports_malformed_and_missing_replies() {
    let listener = HsmsListener::bind(("127.0.0.1", 0), HsmsConfig::default())
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let accept =
        tokio::spawn(async move { GemHost::accept(&listener, IdFormat::U4).await.unwrap() });
    let (equipment, mut erx) = HsmsConnection::connect(("127.0.0.1", port), HsmsConfig::default())
        .await
        .unwrap();
    let host = accept.await.unwrap();

    // answer S1F3 with garbage, S1F1 with the wrong function, S1F11 with no data
    let link = equipment.link();
    tokio::spawn(async move {
        while let Some(inc) = erx.recv().await {
            let reply = match (inc.stream(), inc.function()) {
                (1, 3) => SecsMessage::notify(1, 4, Some(Item::u1(3))),
                (1, 1) => SecsMessage::notify(1, 4, Some(Item::empty_list())),
                (1, 11) => SecsMessage::notify(1, 12, None),
                _ => continue,
            };
            link.reply(&inc, reply).unwrap();
        }
    });
    assert!(matches!(
        host.status_variables(&[]).await,
        Err(SecsError::UnexpectedReply { call: "S1F4", .. })
    ));
    assert!(matches!(
        host.are_you_there().await,
        Err(SecsError::UnexpectedReply { call: "S1F1", .. })
    ));
    assert!(matches!(
        host.status_variable_names(&[]).await,
        Err(SecsError::UnexpectedReply { call: "S1F11", .. })
    ));

    // equipment that never answers: T3 by name, after the configured time
    let silent = HsmsListener::bind(("127.0.0.1", 0), HsmsConfig::default())
        .await
        .unwrap();
    let silent_port = silent.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (_conn, mut rx, _) = silent.accept().await.unwrap();
        while rx.recv().await.is_some() {}
    });
    let cfg = GemHostConfig {
        address: format!("127.0.0.1:{silent_port}"),
        hsms: HsmsConfig {
            t3_ms: 300,
            ..Default::default()
        },
        ..Default::default()
    };
    let host = GemHost::connect(cfg).await.unwrap();
    let start = std::time::Instant::now();
    assert_eq!(
        host.equipment_constants(&[]).await,
        Err(SecsError::Timeout(Timer::T3))
    );
    assert!(start.elapsed() >= Duration::from_millis(300));
    // the session is still usable after a T3
    assert!(host.connection().is_selected());
}

/// secsgem leaves the W-bit clear on S5F3, S5F1 and others and still waits for the reply, so both
/// sides answer recognized messages either way.
#[tokio::test]
async fn replies_are_sent_even_without_the_w_bit() {
    // equipment side
    let sim = EquipmentSimulator::start(sim_config()).await.unwrap();
    let mut log = sim.subscribe_log();
    let (conn, _rx) = HsmsConnection::connect(("127.0.0.1", sim.port()), HsmsConfig::default())
        .await
        .unwrap();
    let s5f3 = SecsMessage::from_sml("S5F3 <L [2] <B 0x00> <U4 3>>.").unwrap();
    assert!(!s5f3.w_bit);
    conn.send(s5f3).unwrap();
    loop {
        let e = next(&mut log).await;
        if e.direction == Direction::Tx && e.kind == "S5F4" {
            assert!(e.text.contains("<B 0x00>"), "{}", e.text);
            break;
        }
    }
    assert!(!sim.model().alarms[2].enabled);

    // host side
    let listener = HsmsListener::bind(("127.0.0.1", 0), HsmsConfig::default())
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let accept =
        tokio::spawn(async move { GemHost::accept(&listener, IdFormat::U4).await.unwrap() });
    let (equipment, _erx) = HsmsConnection::connect(("127.0.0.1", port), HsmsConfig::default())
        .await
        .unwrap();
    let host = accept.await.unwrap();
    let mut events = host.subscribe();
    let mut elog = equipment.subscribe_log();
    equipment
        .send(SecsMessage::from_sml("S5F1 <L [3] <B 0x81> <U4 9> <A \"x\">>.").unwrap())
        .unwrap();
    assert_eq!(next_alarm(&mut events).await.alid, 9);
    loop {
        let e = next(&mut elog).await;
        if e.direction == Direction::Rx && e.kind == "S5F2" {
            break;
        }
    }
    drop(host);
}
