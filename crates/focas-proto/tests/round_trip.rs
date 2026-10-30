//! Client <-> simulator over real TCP on loopback: every call the client implements.

use std::time::Duration;

use focas_proto::codec::{self, Command, DEVICE_CNC, SubRequest};
use focas_proto::functions::{self, req_position, req_sequence_number, req_statinfo};
use focas_proto::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn start() -> (FocasSimulator, FocasClient) {
    start_with(CncProfile::Series31iB).await
}

async fn start_with(profile: CncProfile) -> (FocasSimulator, FocasClient) {
    let sim = FocasSimulator::start(SimulatorConfig {
        port: 0,
        profile,
        tick: None,
        ..Default::default()
    })
    .await
    .expect("simulator starts");
    let client = FocasClient::connect(FocasConfig::new("127.0.0.1").with_port(sim.port()))
        .await
        .expect("client connects");
    (sim, client)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[tokio::test]
async fn connect_reads_sysinfo_and_disconnects() {
    let (sim, client) = start().await;
    assert!(client.is_connected());
    assert_eq!(sim.active_sessions(), 1);

    let cached = client.cached_sysinfo().expect("read at connect");
    let fresh = client.sysinfo().await.unwrap();
    assert_eq!(cached, fresh);
    assert_eq!(fresh, sim.model().sysinfo);
    assert_eq!(fresh.cnc_type, "31");
    assert_eq!(fresh.mt_type, "M");
    assert_eq!(fresh.max_axis, 32);
    assert_eq!(fresh.axis_count(), Some(3));
    assert!(fresh.is_i_series());

    client.disconnect().await.unwrap();
    assert!(!client.is_connected());
    assert!(matches!(
        client.statinfo().await,
        Err(FocasError::NotConnected)
    ));
    // second disconnect is a no-op
    client.disconnect().await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(sim.active_sessions(), 0);
    sim.stop().await;
}

#[tokio::test]
async fn zero_i_f_profile_has_its_own_identity() {
    let (sim, client) = start_with(CncProfile::Series0iF).await;
    let info = client.cached_sysinfo().unwrap();
    assert_eq!(info.cnc_type, "0");
    assert_eq!(info.series, "D4F1");
    assert_eq!(info.max_axis, 8);
    // parameter payloads follow the profile's max_axis
    let p = client.read_params(1020, 1020, -1).await.unwrap();
    assert_eq!(p[0].values, vec![88, 89, 90]);
    sim.stop().await;
}

#[tokio::test]
async fn statinfo_follows_the_model() {
    let (sim, client) = start().await;
    let s = client.statinfo().await.unwrap();
    assert_eq!(s.aut, AutoMode::Memory);
    assert_eq!(s.run, RunState::Reset);
    assert_eq!(s.motion, MotionState::None);
    assert_eq!(s.emergency, EmergencyState::Normal);
    assert_eq!(s.alarm, AlarmState::None);
    assert_eq!(s.edit, EditState::NotEditing);

    sim.with_model(|m| {
        assert!(m.cycle_start());
        m.mode = AutoMode::Jog;
        m.edit = EditState::Searching;
    });
    sim.advance(Duration::from_millis(500));
    let s = client.statinfo().await.unwrap();
    assert_eq!(s.aut, AutoMode::Jog);
    assert_eq!(s.run, RunState::Start);
    assert_eq!(s.motion, MotionState::Motion);
    assert_eq!(s.edit, EditState::Searching);
    assert_eq!(s.run.label(), "STRT");

    sim.with_model(|m| m.set_emergency(true));
    let s = client.statinfo().await.unwrap();
    assert_eq!(s.emergency, EmergencyState::Emergency);
    assert_eq!(s.run, RunState::Reset);
    sim.stop().await;
}

#[tokio::test]
async fn axes_and_positions() {
    let (sim, client) = start().await;
    let names = client.axis_names().await.unwrap();
    assert_eq!(
        names.iter().map(|n| n.to_string()).collect::<Vec<_>>(),
        ["X", "Y", "Z"]
    );

    sim.with_model(|m| {
        m.axes[0].absolute = 12.345;
        m.axes[0].work_offset = -250.0;
        m.axes[0].relative_origin = 2.0;
        m.axes[0].distance_to_go = -1.5;
        m.axes[1].absolute = -0.001;
        m.axes[2].absolute = 150.0;
        m.axes[2].skip_position = 7.25;
    });
    let abs = client.read_position(PositionKind::Absolute).await.unwrap();
    assert_eq!(abs.len(), 3, "slots of unused axes are cut off");
    assert!(close(abs[0].unwrap(), 12.345));
    assert!(close(abs[1].unwrap(), -0.001));
    assert!(close(abs[2].unwrap(), 150.0));

    let all = client.read_positions().await.unwrap();
    assert_eq!(all.axes.value().unwrap().len(), 3);
    let machine = all.machine.value().unwrap();
    assert!(close(machine[0].unwrap(), 12.345 - 250.0));
    assert!(close(all.relative.value().unwrap()[0].unwrap(), 10.345));
    assert!(close(all.distance_to_go.value().unwrap()[0].unwrap(), -1.5));
    assert!(close(
        client.read_position(PositionKind::Skip).await.unwrap()[2].unwrap(),
        7.25
    ));
    sim.stop().await;
}

#[tokio::test]
async fn feed_spindle_and_servo_load_while_the_cycle_runs() {
    let (sim, client) = start().await;
    assert_eq!(client.actual_feed().await.unwrap(), Some(0.0));
    assert_eq!(client.spindle_speed().await.unwrap(), Some(0.0));

    sim.with_model(|m| assert!(m.cycle_start()));
    // N10 is a rapid to Z50 with the spindle starting; the spindle ramps up at 6000 rpm/s
    sim.advance(Duration::from_millis(300));
    let rpm = client.spindle_speed().await.unwrap().unwrap();
    assert!(rpm > 1000.0 && rpm <= 3000.0, "rpm {rpm}");

    // advance into the cutting move N30 (G01 Z-2 F200)
    sim.with_model(|m| {
        m.block_index = 2;
        m.axes[2].absolute = 5.0;
    });
    sim.advance(Duration::from_millis(100));
    assert_eq!(client.actual_feed().await.unwrap(), Some(200.0));
    let load = client.servo_load().await.unwrap();
    assert_eq!(load.len(), 3);
    assert!(load[2].unwrap() > load[0].unwrap(), "{load:?}");
    sim.stop().await;
}

#[tokio::test]
async fn program_calls() {
    let (sim, client) = start().await;
    let n = client.program_numbers().await.unwrap();
    assert_eq!((n.running, n.main), (3000, 3000));
    assert_eq!(client.sequence_number().await.unwrap(), 10);
    assert_eq!(
        client.main_program_name().await.unwrap(),
        "//CNC_MEM/USER/PATH1/O3000"
    );

    sim.with_model(|m| m.block_index = 3);
    assert_eq!(client.sequence_number().await.unwrap(), 40);
    let block = client.executing_block(256).await.unwrap();
    assert_eq!(block.block, 3);
    assert_eq!(block.text, "N40 G01 X60. F600");
    // text is cut at the requested length
    assert_eq!(client.executing_block(3).await.unwrap().text, "N40");

    let programs = client.list_programs().await.unwrap();
    let numbers: Vec<u32> = programs.iter().map(|p| p.number).collect();
    assert_eq!(numbers, [3000, 3001]);
    assert_eq!(programs[0].comment, "POCKET DEMO");
    assert_eq!(programs[0].size, sim.model().programs[&3000].size());

    // more programs than one directory batch (19) are paged
    sim.with_model(|m| {
        for i in 1..=45u32 {
            m.programs.insert(
                i,
                SimProgram {
                    comment: format!("P{i}"),
                    blocks: Vec::new(),
                },
            );
        }
    });
    let programs = client.list_programs().await.unwrap();
    assert_eq!(programs.len(), 47);
    assert!(programs.windows(2).all(|w| w[0].number < w[1].number));
    sim.stop().await;
}

#[tokio::test]
async fn alarms_raise_and_clear() {
    let (sim, client) = start().await;
    assert_eq!(client.alarm_bits().await.unwrap(), AlarmBits(0));
    assert!(
        client
            .alarm_messages(AlarmQuery::default())
            .await
            .unwrap()
            .is_empty()
    );

    sim.with_model(|m| {
        m.raise_alarm(401, AlarmType::Servo, 2, "SERVO ALARM: VRDY OFF");
        m.raise_alarm(
            100,
            AlarmType::ParameterSwitchOn,
            0,
            "PARAMETER WRITE ENABLE",
        );
    });
    let bits = client.alarm_bits().await.unwrap();
    assert!(bits.is_set(6) && bits.is_set(0));
    assert_eq!(
        bits.types(),
        [AlarmType::ParameterSwitchOn, AlarmType::Servo]
    );
    assert_eq!(client.statinfo().await.unwrap().alarm, AlarmState::Alarm);

    let msgs = client.alarm_messages(AlarmQuery::default()).await.unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].number, 401);
    assert_eq!(msgs[0].alarm_type, AlarmType::Servo);
    assert_eq!(msgs[0].axis, 2);
    assert_eq!(msgs[0].text.as_deref(), Some("SERVO ALARM: VRDY OFF"));
    assert_eq!(msgs[1].text.as_deref(), Some("PARAMETER WRITE ENABLE"));

    // filter by type, no text, message limit
    let only_servo = client
        .alarm_messages(AlarmQuery {
            alarm_type: 6,
            with_text: false,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(only_servo.len(), 1);
    assert_eq!(only_servo[0].text, None);
    let one = client
        .alarm_messages(AlarmQuery {
            max_messages: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(one.len(), 1);
    // a wrong type is an attribute error, a zero count a length error
    assert!(matches!(
        client
            .alarm_messages(AlarmQuery {
                alarm_type: 77,
                ..Default::default()
            })
            .await,
        Err(FocasError::Cnc { code: 4, .. })
    ));
    assert!(matches!(
        client
            .alarm_messages(AlarmQuery {
                max_messages: 0,
                ..Default::default()
            })
            .await,
        Err(FocasError::InvalidArgument(_))
    ));

    sim.with_model(|m| m.clear_alarms());
    assert_eq!(client.alarm_bits().await.unwrap(), AlarmBits(0));
    assert_eq!(client.statinfo().await.unwrap().alarm, AlarmState::None);
    sim.stop().await;
}

#[tokio::test]
async fn macro_variables_read_and_write() {
    let (sim, client) = start().await;
    let v = client.read_macro(500, 502).await.unwrap();
    assert_eq!(
        v.iter().map(|m| m.number).collect::<Vec<_>>(),
        [500, 501, 502]
    );
    assert_eq!(v[0].value, Some(0.0));
    assert!(close(v[1].value.unwrap(), 1250.5));
    assert!(close(v[2].value.unwrap(), 100.0));

    // a vacant variable reads as invalid
    let vacant = client.read_macro(700, 700).await.unwrap();
    assert_eq!(vacant[0].value, None);

    client.write_macro(510, -7.654321).await.unwrap();
    assert!(close(
        client.read_macro(510, 510).await.unwrap()[0].value.unwrap(),
        -7.654321
    ));
    assert!(close(sim.model().macros[&510], -7.654321));

    assert!(matches!(
        client.read_macro(10, 5).await,
        Err(FocasError::InvalidArgument(_))
    ));
    assert!(matches!(
        client.write_macro(0, 1.0).await,
        Err(FocasError::InvalidArgument(_))
    ));
    assert!(matches!(
        client.write_macro(1, f64::NAN).await,
        Err(FocasError::InvalidArgument(_))
    ));

    // the part counter macro counts finished cycles
    sim.with_model(|m| m.cycle_start());
    for _ in 0..4000 {
        sim.advance(Duration::from_millis(100));
        if sim.model().run == RunState::Reset {
            break;
        }
    }
    assert_eq!(
        client.read_macro(500, 500).await.unwrap()[0].value,
        Some(1.0)
    );
    sim.stop().await;
}

#[tokio::test]
async fn parameters_and_diagnosis() {
    let (sim, client) = start().await;
    // byte parameter, all axes: one value per controlled axis, not per max_axis slot
    let p = client.read_params(1020, 1020, -1).await.unwrap();
    assert_eq!(p.len(), 1);
    assert_eq!((p[0].number, p[0].axis, p[0].data_type), (1020, -1, 0));
    assert_eq!(p[0].values, vec![88, 89, 90]);
    // one axis
    let p = client.read_params(1320, 1320, 2).await.unwrap();
    assert_eq!((p[0].axis, p[0].values.clone()), (2, vec![200_000]));
    // a range, int and bit types
    let p = client.read_params(1320, 1321, 3).await.unwrap();
    assert_eq!(p[0].values, vec![100_000]);
    assert_eq!(p[1].values, vec![-100_000]);
    let bit = client.read_params(3401, 3401, 0).await.unwrap();
    assert_eq!((bit[0].data_type, bit[0].values.clone()), (1, vec![1]));
    // short type
    sim.with_model(|m| {
        m.params.insert(
            1815,
            SimNumbered {
                data_type: 2,
                values: vec![-2, 300, 7],
            },
        );
    });
    let s = client.read_params(1815, 1815, -1).await.unwrap();
    assert_eq!(s[0].values, vec![-2, 300, 7]);

    // part count parameters follow the counter
    assert_eq!(
        client.read_params(6711, 6712, 0).await.unwrap()[0].values,
        vec![0]
    );
    // missing number -> EW_NUMBER, bad axis -> EW_NUMBER
    assert!(matches!(
        client.read_params(9999, 9999, 0).await,
        Err(FocasError::Cnc { code: 3, .. })
    ));
    assert!(matches!(
        client.read_params(1320, 1320, 9).await,
        Err(FocasError::Cnc { code: 3, .. })
    ));
    assert!(matches!(
        client.read_params(5, 1, 0).await,
        Err(FocasError::InvalidArgument(_))
    ));

    let d = client.read_diag(308, 309, -1).await.unwrap();
    assert_eq!(d[0].values, vec![28, 28, 44]);
    assert_eq!(d[1].values, vec![36, 36, 49]);
    assert_eq!(d[0].data_type, 2);
    sim.stop().await;
}

#[tokio::test]
async fn pmc_reads_every_width() {
    let (sim, client) = start().await;
    let w = client
        .read_pmc(PmcArea::D, PmcDataType::Word, 2204, 1)
        .await
        .unwrap();
    assert_eq!(
        w,
        vec![PmcValue {
            address: 2204,
            value: 600
        }]
    );
    let d = client
        .read_pmc(PmcArea::D, PmcDataType::DWord, 1870, 2)
        .await
        .unwrap();
    assert_eq!(
        d,
        vec![
            PmcValue {
                address: 1870,
                value: 100
            },
            PmcValue {
                address: 1874,
                value: 500
            }
        ]
    );
    sim.with_model(|m| {
        m.set_pmc(PmcArea::R, PmcDataType::Byte, 100, 0xa5);
        m.set_pmc(PmcArea::R, PmcDataType::Word, 102, 0xbeef);
    });
    let b = client
        .read_pmc(PmcArea::R, PmcDataType::Byte, 100, 4)
        .await
        .unwrap();
    assert_eq!(
        b.iter().map(|v| (v.address, v.value)).collect::<Vec<_>>(),
        [(100, 0xa5), (101, 0), (102, 0xbe), (103, 0xef)]
    );
    let w = client
        .read_pmc(PmcArea::R, PmcDataType::Word, 102, 1)
        .await
        .unwrap();
    assert_eq!(w[0].value, 0xbeef);
    assert_eq!(
        client
            .read_pmc(PmcArea::X, PmcDataType::Byte, 0, 1)
            .await
            .unwrap()[0]
            .value,
        1
    );
    // beyond the area
    assert!(matches!(
        client.read_pmc(PmcArea::A, PmcDataType::Byte, 49, 2).await,
        Err(FocasError::Cnc { code: 3, .. })
    ));
    assert!(matches!(
        client.read_pmc(PmcArea::D, PmcDataType::Byte, 0, 0).await,
        Err(FocasError::InvalidArgument(_))
    ));
    sim.stop().await;
}

#[tokio::test]
async fn snapshot_is_one_poll() {
    let (sim, client) = start().await;
    sim.with_model(|m| {
        m.cycle_start();
        m.axes[0].absolute = 10.0;
    });
    sim.advance(Duration::from_millis(100));
    let before = sim.requests_served();
    let snap = client.snapshot().await.unwrap();
    // 6 items + names + 4 positions = 11 sub-packets (two round trips of at most nine)
    assert_eq!(sim.requests_served() - before, 11);
    assert_eq!(snap.status.value().unwrap().run, RunState::Start);
    assert_eq!(snap.programs.value().unwrap().main, 3000);
    assert_eq!(
        *snap.sequence_number.value().unwrap(),
        sim.model().sequence_number()
    );
    assert_eq!(snap.alarm_bits.value(), Some(&AlarmBits(0)));
    assert!(snap.feed_rate.is_value() && snap.spindle_speed.is_value());
    assert_eq!(snap.positions.absolute.value().unwrap().len(), 3);
    assert_eq!(snap.positions.axes.value().unwrap().len(), 3);
    // it serializes for a UI
    let json = serde_json::to_value(&snap).unwrap();
    assert_eq!(json["status"]["status"], "value");
    assert_eq!(json["status"]["data"]["run"], "start");
    sim.stop().await;
}

#[tokio::test]
async fn refused_items_do_not_fail_the_snapshot() {
    // A sub-packet the CNC refuses (like the sample's spindle read answering EW_FUNC) must stay
    // local. Use the raw request path to show the shape, then the typed snapshot against a
    // simulator whose model is fine.
    let (sim, client) = start().await;
    let responses = client
        .request(&[
            req_statinfo(),
            SubRequest::new(Command::cnc(0x7777), [0; 5]),
            req_position(PositionKind::Absolute, 2), // a single axis is not simulated
        ])
        .await
        .unwrap();
    assert_eq!(responses[0].error, 0);
    assert_eq!(responses[1].error, ew::FUNC);
    assert_eq!(responses[2].error, ew::NUMBER);
    assert!(responses[1].payload.is_empty());
    sim.stop().await;
}

#[tokio::test]
async fn long_multi_requests_are_split() {
    let (sim, client) = start().await;
    let subs: Vec<SubRequest> = (0..25).map(|_| req_sequence_number()).collect();
    let before = sim.requests_served();
    let responses = client.request(&subs).await.unwrap();
    assert_eq!(responses.len(), 25);
    assert!(
        responses
            .iter()
            .all(|r| r.error == 0 && r.payload.len() == 4)
    );
    assert_eq!(sim.requests_served() - before, 25);
    assert!(matches!(
        client.request(&[]).await,
        Err(FocasError::InvalidArgument(_))
    ));
    sim.stop().await;
}

#[tokio::test]
async fn background_tick_moves_the_machine() {
    let sim = FocasSimulator::start(SimulatorConfig {
        port: 0,
        tick: Some(Duration::from_millis(10)),
        ..Default::default()
    })
    .await
    .unwrap();
    let client = FocasClient::connect(FocasConfig::new("127.0.0.1").with_port(sim.port()))
        .await
        .unwrap();
    let z0 = client.read_position(PositionKind::Absolute).await.unwrap()[2].unwrap();
    sim.with_model(|m| assert!(m.cycle_start()));
    tokio::time::sleep(Duration::from_millis(400)).await;
    let z1 = client.read_position(PositionKind::Absolute).await.unwrap()[2].unwrap();
    assert!(z1 < z0, "Z should have moved down from {z0}, is {z1}");
    assert_eq!(client.statinfo().await.unwrap().run, RunState::Start);
    sim.stop().await;
}

#[tokio::test]
async fn stopping_the_simulator_ends_sessions() {
    let (sim, client) = start().await;
    client.statinfo().await.unwrap();
    sim.stop().await;
    let err = client.statinfo().await.unwrap_err();
    assert!(
        matches!(err, FocasError::Io(_) | FocasError::Timeout(_)),
        "{err:?}"
    );
    assert!(!client.is_connected());
    assert!(matches!(
        client.statinfo().await,
        Err(FocasError::NotConnected)
    ));
    // nothing listens any more
    let again = FocasClient::connect(FocasConfig::new("127.0.0.1").with_port(sim.port())).await;
    assert!(matches!(again, Err(FocasError::ConnectionFailed(_))));
}

#[tokio::test]
async fn several_clients_share_one_machine() {
    let (sim, a) = start().await;
    let b = FocasClient::connect(FocasConfig::new("127.0.0.1").with_port(sim.port()))
        .await
        .unwrap();
    assert_eq!(sim.active_sessions(), 2);
    a.write_macro(520, 42.0).await.unwrap();
    assert_eq!(b.read_macro(520, 520).await.unwrap()[0].value, Some(42.0));
    sim.stop().await;
}

#[tokio::test]
async fn connect_times_out_against_a_silent_server() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let silent = tokio::spawn(async move {
        let (_s, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    let started = std::time::Instant::now();
    let result = FocasClient::connect(FocasConfig {
        port,
        connect_timeout_ms: 200,
        ..FocasConfig::new("127.0.0.1")
    })
    .await;
    assert!(matches!(result, Err(FocasError::ConnectionFailed(_))));
    assert!(started.elapsed() < Duration::from_secs(2));
    silent.abort();
}

#[tokio::test]
async fn request_timeout_drops_the_connection() {
    // Answers the open request and sysinfo, then goes quiet.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut open = [0u8; 12];
        s.read_exact(&mut open).await.unwrap();
        s.write_all(&codec::Frame::open_response().encode().unwrap())
            .await
            .unwrap();
        let mut request = [0u8; 40];
        s.read_exact(&mut request).await.unwrap();
        let sysinfo = codec::SubResponse::ok(
            functions::func::SYSINFO,
            functions::encode_sysinfo(&CncModel::default().sysinfo),
        );
        s.write_all(
            &codec::encode_var_response(&[sysinfo])
                .unwrap()
                .encode()
                .unwrap(),
        )
        .await
        .unwrap();
        // read the next request and never answer it
        let mut sink = [0u8; 64];
        let _ = s.read(&mut sink).await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    let client = FocasClient::connect(FocasConfig {
        port,
        request_timeout_ms: 150,
        ..FocasConfig::new("127.0.0.1")
    })
    .await
    .unwrap();
    assert_eq!(client.cached_sysinfo().unwrap().cnc_type, "31");
    let err = client.statinfo().await.unwrap_err();
    assert!(matches!(err, FocasError::Timeout(_)), "{err:?}");
    // the stream may hold a late answer, so the connection is gone
    assert!(!client.is_connected());
    assert!(matches!(
        client.statinfo().await,
        Err(FocasError::NotConnected)
    ));
    server.abort();
}

#[tokio::test]
async fn a_server_that_never_answers_sysinfo_fails_the_connect() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut open = [0u8; 12];
        s.read_exact(&mut open).await.unwrap();
        s.write_all(&codec::Frame::open_response().encode().unwrap())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    let result = FocasClient::connect(FocasConfig {
        port,
        request_timeout_ms: 150,
        ..FocasConfig::new("127.0.0.1")
    })
    .await;
    assert!(
        matches!(result, Err(FocasError::ConnectionFailed(_))),
        "{:?}",
        result.err()
    );
    server.abort();
}

/// The nine-sub-packet request of pyfanuc's `sample_with_error.md`, sent byte for byte over a raw
/// socket, and the framing of what comes back.
#[tokio::test]
async fn raw_pyfanuc_sample_request_is_answered_in_one_frame() {
    let (sim, _client) = start().await;
    let mut stream = TcpStream::connect(("127.0.0.1", sim.port())).await.unwrap();
    stream
        .write_all(&[0xa0, 0xa0, 0xa0, 0xa0, 0, 1, 1, 1, 0, 2, 0, 2])
        .await
        .unwrap();
    let mut open = [0u8; 10];
    stream.read_exact(&mut open).await.unwrap();
    assert_eq!(open, [0xa0, 0xa0, 0xa0, 0xa0, 0, 2, 1, 2, 0, 0]);

    let mut req: Vec<u8> = vec![0xa0, 0xa0, 0xa0, 0xa0, 0, 1, 0x21, 1, 0, 0xfe, 0, 9];
    let sub = |func: u8, a1: i32, a2: i32| {
        let mut s = vec![0, 0x1c, 0, 1, 0, 1, 0, func];
        s.extend_from_slice(&a1.to_be_bytes());
        s.extend_from_slice(&a2.to_be_bytes());
        s.extend_from_slice(&[0u8; 12]);
        s
    };
    for s in [
        sub(0x1a, 0, 0),
        sub(0x1c, 0, 0),
        sub(0x1d, 0, 0),
        sub(0x24, 0, 0),
        sub(0x25, 0, 0),
        sub(0x26, 4, -1),
        sub(0x26, 1, -1),
        sub(0x26, 6, -1),
        sub(0x26, 7, -1),
    ] {
        req.extend(s);
    }
    stream.write_all(&req).await.unwrap();

    let mut head = [0u8; 10];
    stream.read_exact(&mut head).await.unwrap();
    assert_eq!(&head[..4], &[0xa0; 4]);
    assert_eq!(&head[6..8], &[0x21, 0x02]);
    let len = usize::from(u16::from_be_bytes([head[8], head[9]]));
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.unwrap();
    // nothing else is pending: the whole answer was one frame
    let mut extra = [0u8; 1];
    assert!(
        tokio::time::timeout(Duration::from_millis(100), stream.read(&mut extra))
            .await
            .is_err()
    );

    let mut frame = head.to_vec();
    frame.extend(body);
    let subs = codec::decode_var_response(&codec::Frame::decode(&frame).unwrap()).unwrap();
    assert_eq!(subs.len(), 9);
    assert!(
        subs.iter()
            .all(|s| s.error == 0 && s.command.device == DEVICE_CNC)
    );
    assert_eq!(subs[1].payload, [0, 0, 0x0b, 0xb8, 0, 0, 0x0b, 0xb8]); // O3000 running and main
    assert_eq!(subs[5].payload.len(), 64); // eight 8-byte axis slots
    // 3 axes: Z absolute is 50.000 with three decimals
    assert_eq!(&subs[5].payload[16..24], &[0, 0, 0xc3, 0x50, 0, 10, 0, 3]);

    // close handshake
    stream
        .write_all(&[0xa0, 0xa0, 0xa0, 0xa0, 0, 1, 2, 1, 0, 0])
        .await
        .unwrap();
    let mut closed = [0u8; 10];
    stream.read_exact(&mut closed).await.unwrap();
    assert_eq!(closed, [0xa0, 0xa0, 0xa0, 0xa0, 0, 2, 2, 2, 0, 0]);
    sim.stop().await;
}

#[tokio::test]
async fn garbage_closes_only_that_session() {
    let (sim, client) = start().await;
    let mut bad = TcpStream::connect(("127.0.0.1", sim.port())).await.unwrap();
    bad.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
    let mut buf = [0u8; 16];
    let n = tokio::time::timeout(Duration::from_secs(2), bad.read(&mut buf))
        .await
        .expect("server reacts")
        .unwrap_or(0);
    assert_eq!(n, 0, "no answer to garbage, connection closed");
    assert!(client.statinfo().await.is_ok());
    sim.stop().await;
}

#[tokio::test]
async fn extended_snapshot_reads_servo_name_and_macros() {
    let (sim, client) = start().await;
    sim.with_model(|m| {
        m.cycle_start();
    });
    sim.advance(Duration::from_millis(1500));
    let extra = client
        .extended_snapshot(&ExtraQuery {
            servo_load: true,
            program_name: true,
            macros: vec![500, 501, 9999],
        })
        .await
        .unwrap();
    let load = extra.servo_load.unwrap().into_result().unwrap();
    assert_eq!(load.len(), 32);
    assert!(load[0].is_some());
    assert_eq!(
        extra.program_name.unwrap().into_result().unwrap(),
        "//CNC_MEM/USER/PATH1/O3000"
    );
    assert_eq!(extra.macros.len(), 3);
    assert_eq!(extra.macros[1].value().unwrap().value, Some(1250.5));
    // a vacant variable is a value of `None`, not a failure
    assert_eq!(extra.macros[2].value().unwrap().value, None);

    // nothing asked, nothing sent
    let before = sim.requests_served();
    let none = client
        .extended_snapshot(&ExtraQuery::default())
        .await
        .unwrap();
    assert!(none.servo_load.is_none() && none.program_name.is_none() && none.macros.is_empty());
    assert_eq!(sim.requests_served(), before);
    assert!(matches!(
        client
            .extended_snapshot(&ExtraQuery {
                macros: vec![0],
                ..Default::default()
            })
            .await,
        Err(FocasError::InvalidArgument(_))
    ));
    sim.stop().await;
}

#[tokio::test]
async fn a_refused_item_fails_alone() {
    let (sim, client) = start().await;
    sim.with_model(|m| {
        m.refuse
            .insert(functions::func::ACTUAL_SPINDLE.function, ew::NOOPT);
        m.refuse
            .insert(functions::func::PROGRAM_NAME.function, ew::FUNC);
    });
    let snap = client.snapshot().await.unwrap();
    assert!(snap.status.is_value());
    assert!(snap.positions.absolute.is_value());
    match &snap.spindle_speed {
        Reading::Failed(f) => assert_eq!(f.code, Some(ew::NOOPT)),
        other => panic!("spindle should be refused: {other:?}"),
    }
    let extra = client
        .extended_snapshot(&ExtraQuery {
            servo_load: true,
            program_name: true,
            macros: vec![500],
        })
        .await
        .unwrap();
    assert!(extra.servo_load.unwrap().is_value());
    assert!(!extra.program_name.unwrap().is_value());
    assert!(extra.macros[0].is_value());
    sim.stop().await;
}
