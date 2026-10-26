//! The OPC UA plant server and clients talking over loopback through the engine, the way the app
//! drives them: endpoint discovery, anonymous, user-name and certificate sign-in, browsing,
//! monitoring, typed writes (and refusals), holds, and method calls.

use std::path::PathBuf;
use std::sync::Once;
use std::time::{Duration, Instant};

use vt_protocol::{
    Command, Event, EventKind, OpcuaAuth, OpcuaBrowseNode, OpcuaClientSpec, OpcuaEvent, OpcuaNode,
    OpcuaNodeClass, OpcuaRequest, OpcuaSecurityMode, OpcuaSecurityPolicy, OpcuaServerSpec,
    PointInfo, Quality, Reading, SessionId, SessionSpec, SessionState,
};

static PKI: Once = Once::new();

/// Certificates of this test run, away from the user's.
fn pki_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("vt-engine-opcua-{}", std::process::id()));
    PKI.call_once(|| {
        let _ = std::fs::remove_dir_all(&root);
        vt_engine::opcua::set_pki_root(root.clone());
        // The stack's and the session's trace, shown only when a test fails.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new(
                "opcua_client=debug,opcua_core=info,opcua_server=info,vt_engine=trace",
            ))
            .with_test_writer()
            .with_thread_names(true)
            .try_init();
    });
    root
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .expect("a free TCP port")
}

/// The engine's events, kept until a test asks for them: several sessions talk at once.
struct Events {
    receiver: async_channel::Receiver<Event>,
    kept: Vec<Event>,
}

impl Events {
    fn new(receiver: async_channel::Receiver<Event>) -> Self {
        Self {
            receiver,
            kept: Vec::new(),
        }
    }

    /// The first event of `session` (kept or new) that `accept` maps to a value; it and every
    /// earlier event of the session are consumed. Key generation in a debug build is slow, so
    /// the wait is generous.
    #[track_caller]
    fn wait<T>(
        &mut self,
        session: SessionId,
        mut accept: impl FnMut(&EventKind) -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut scanned = 0;
        loop {
            while scanned < self.kept.len() {
                let event = &self.kept[scanned];
                if event.session == session
                    && let Some(found) = accept(&event.kind)
                {
                    let mut index = 0;
                    self.kept.retain(|event| {
                        index += 1;
                        index > scanned + 1 || event.session != session
                    });
                    return found;
                }
                scanned += 1;
            }
            if Instant::now() >= deadline {
                // What the session did say: its logs, states and answers, not its values.
                let said: Vec<String> = self
                    .kept
                    .iter()
                    .filter(|event| {
                        !matches!(event.kind, EventKind::Values(_) | EventKind::Stats(_))
                    })
                    .map(|event| format!("{} {:?}", event.session, event.kind))
                    .collect();
                // And how far each session's values got: frames since, and the last one.
                let mut values: Vec<String> = Vec::new();
                for event in &self.kept {
                    if let EventKind::Values(frame) = &event.kind {
                        let readings: Vec<String> = frame
                            .readings
                            .iter()
                            .map(|reading| format!("{:.1}/{:?}", reading.value, reading.quality))
                            .collect();
                        values.push(format!(
                            "{} at {} {}",
                            event.session,
                            frame.at_ms,
                            readings.join(" ")
                        ));
                    }
                }
                let tail = values.len().saturating_sub(6);
                panic!(
                    "timed out waiting for an event from {session}; it said:\n{}\n{} value frames, the last:\n{}",
                    said.join("\n"),
                    values.len(),
                    values[tail..].join("\n")
                );
            }
            match self.receiver.try_recv() {
                Ok(event) => self.kept.push(event),
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }
}

#[track_caller]
fn state(events: &mut Events, session: SessionId) -> SessionState {
    events.wait(session, |kind| match kind {
        EventKind::State(
            state @ (SessionState::Connected
            | SessionState::Listening { .. }
            | SessionState::Reconnecting { .. }
            | SessionState::Closed { .. }),
        ) => Some(state.clone()),
        _ => None,
    })
}

#[track_caller]
fn opcua(events: &mut Events, session: SessionId) -> OpcuaEvent {
    events.wait(session, |kind| match kind {
        EventKind::Opcua(event) => Some(event.clone()),
        _ => None,
    })
}

#[track_caller]
fn points(events: &mut Events, session: SessionId) -> Vec<PointInfo> {
    events.wait(session, |kind| match kind {
        EventKind::Points(points) => Some(points.clone()),
        _ => None,
    })
}

#[track_caller]
fn browse(
    events: &mut Events,
    engine: &vt_engine::EngineHandle,
    session: SessionId,
    node_id: &str,
) -> Vec<OpcuaBrowseNode> {
    assert!(engine.send(Command::Opcua {
        session,
        request: OpcuaRequest::Browse {
            node_id: node_id.into(),
        },
    }));
    match opcua(events, session) {
        OpcuaEvent::Children { parent, result } => {
            assert_eq!(parent, node_id);
            result.unwrap_or_else(|error| panic!("browse {node_id}: {error}"))
        }
        other => panic!("expected children of {node_id}, got {other:?}"),
    }
}

#[track_caller]
fn written(events: &mut Events, session: SessionId, point: u32) -> Result<(), String> {
    events.wait(session, |kind| match kind {
        EventKind::Written {
            point: written,
            result,
        } if *written == point => Some(result.clone()),
        _ => None,
    })
}

/// Wait until point `point` of `session` reads `accept`.
#[track_caller]
fn reads(
    events: &mut Events,
    session: SessionId,
    point: usize,
    mut accept: impl FnMut(Reading) -> bool,
) {
    events.wait(session, |kind| match kind {
        EventKind::Values(frame) => frame
            .readings
            .get(point)
            .copied()
            .filter(|reading| accept(*reading))
            .map(|_| ()),
        _ => None,
    });
}

/// Wait until the payload (the text behind the value) of `point` starts with `prefix`.
#[track_caller]
fn says(events: &mut Events, session: SessionId, point: u32, prefix: &str) {
    events.wait(session, |kind| match kind {
        EventKind::Payloads(texts) => texts
            .iter()
            .any(|text| text.point == point && text.text.starts_with(prefix))
            .then_some(()),
        _ => None,
    });
}

fn client(port: u16, nodes: Vec<OpcuaNode>) -> OpcuaClientSpec {
    OpcuaClientSpec {
        publishing_ms: 100,
        sampling_ms: 50,
        ..OpcuaServerSpec {
            port,
            ..OpcuaServerSpec::default()
        }
        .client(nodes)
    }
}

#[test]
fn clients_discover_browse_monitor_write_and_call_the_plant_server() {
    let root = pki_root();
    let (engine, receiver) = vt_engine::start().expect("the engine");
    let mut events = Events::new(receiver);
    let port = free_port();
    let server = SessionId(1);
    assert!(engine.send(Command::Open {
        session: server,
        spec: SessionSpec::OpcuaServer(OpcuaServerSpec {
            port,
            ..OpcuaServerSpec::default()
        }),
    }));
    let listening = state(&mut events, server);
    assert!(
        matches!(&listening, SessionState::Listening { address } if address.ends_with(&format!(":{port}"))),
        "{listening:?}"
    );
    let served = points(&mut events, server);
    assert_eq!(served.len(), 17);
    assert_eq!(served[0].name, "Reactor › Temperature");
    // Node ids in the plant's namespace, wherever the server put it.
    let prefix = served[0]
        .address
        .strip_suffix("Plant.Reactor.Temperature")
        .expect("a string node id")
        .to_owned();
    assert!(prefix.starts_with("ns="), "{prefix}");
    let node = |key: &str| format!("{prefix}{key}");

    // Discovery needs no session: the answer comes back under the request's id.
    let request = SessionId(100);
    assert!(engine.send(Command::Opcua {
        session: request,
        request: OpcuaRequest::Endpoints {
            url: format!("opc.tcp://localhost:{port}"),
        },
    }));
    let OpcuaEvent::Endpoints(Ok(endpoints)) = opcua(&mut events, request) else {
        panic!("endpoints");
    };
    assert!(
        endpoints
            .iter()
            .any(|endpoint| endpoint.policy == Some(OpcuaSecurityPolicy::None))
    );
    assert!(endpoints.iter().any(|endpoint| {
        endpoint.policy == Some(OpcuaSecurityPolicy::Aes256Sha256RsaPss)
            && endpoint.mode == OpcuaSecurityMode::SignAndEncrypt
    }));
    let tokens = &endpoints[0].user_tokens;
    for token in ["Anonymous", "UserName", "Certificate"] {
        assert!(
            tokens.iter().any(|t| t == token),
            "{token} missing from {tokens:?}"
        );
    }
    assert!(
        endpoints
            .windows(2)
            .all(|pair| pair[0].security_level >= pair[1].security_level),
        "most secure first"
    );
    // Nobody listens on a port the test just freed.
    let nobody = SessionId(101);
    assert!(engine.send(Command::Opcua {
        session: nobody,
        request: OpcuaRequest::Endpoints {
            url: format!("opc.tcp://127.0.0.1:{}", free_port()),
        },
    }));
    assert!(matches!(
        opcua(&mut events, nobody),
        OpcuaEvent::Endpoints(Err(_))
    ));

    // An anonymous client on the None endpoint, monitoring the reactor temperature.
    let anonymous = SessionId(2);
    assert!(engine.send(Command::Open {
        session: anonymous,
        spec: SessionSpec::OpcuaClient(client(
            port,
            vec![OpcuaNode::new(node("Plant.Reactor.Temperature"), "")],
        )),
    }));
    let monitored = points(&mut events, anonymous);
    assert_eq!(state(&mut events, anonymous), SessionState::Connected);
    assert_eq!(monitored.len(), 1);
    assert_eq!(
        monitored[0].name, "Temperature",
        "the display name, as none was given"
    );
    assert_eq!(monitored[0].unit, "°C");
    assert_eq!(monitored[0].data_type, "Double");
    assert!(monitored[0].writable);
    reads(&mut events, anonymous, 0, |reading| {
        reading.quality == Quality::Good && reading.value.is_finite()
    });

    // The address space, a level at a time.
    let objects = browse(&mut events, &engine, anonymous, "Objects");
    let plant = objects
        .iter()
        .find(|child| child.display_name == "Plant")
        .expect("the plant folder");
    assert!(plant.has_children);
    let machines = browse(&mut events, &engine, anonymous, &plant.node_id);
    let mut names: Vec<&str> = machines.iter().map(|m| m.display_name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Boiler", "Reactor", "Utilities"]);
    let reactor = browse(&mut events, &engine, anonymous, &node("Plant.Reactor"));
    let start = reactor
        .iter()
        .find(|child| child.display_name == "StartBatch")
        .expect("the StartBatch method");
    assert_eq!(start.class, OpcuaNodeClass::Method);
    let temperature = reactor
        .iter()
        .find(|child| child.display_name == "Temperature")
        .expect("the temperature variable");
    assert_eq!(temperature.class, OpcuaNodeClass::Variable);
    assert_eq!(temperature.data_type, "Double");
    assert!(temperature.has_children, "its EURange and EngineeringUnits");
    let properties = browse(&mut events, &engine, anonymous, &temperature.node_id);
    assert!(
        properties
            .iter()
            .any(|p| p.browse_name == "EngineeringUnits")
    );
    assert!(!properties.iter().any(|p| p.has_children));

    // Attributes for the inspector.
    assert!(engine.send(Command::Opcua {
        session: anonymous,
        request: OpcuaRequest::Attributes {
            node_id: node("Plant.Reactor.Setpoint"),
        },
    }));
    let OpcuaEvent::Attributes {
        result: Ok(attributes),
        ..
    } = opcua(&mut events, anonymous)
    else {
        panic!("attributes");
    };
    let attribute = |name: &str| {
        attributes
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.clone())
            .unwrap_or_default()
    };
    assert_eq!(attribute("DisplayName"), "Setpoint");
    assert_eq!(attribute("AccessLevel"), "3 (read, write)");

    // More variables join the points; an unknown one is refused, a known one is not added twice.
    assert!(engine.send(Command::Opcua {
        session: anonymous,
        request: OpcuaRequest::Monitor {
            nodes: vec![
                OpcuaNode::new(node("Plant.Reactor.Setpoint"), "Reactor › Setpoint"),
                OpcuaNode::new(node("Plant.Reactor.Recipe"), ""),
                OpcuaNode::new(node("Plant.Reactor.BatchId"), ""),
                OpcuaNode::new(node("Plant.Nowhere"), ""),
                OpcuaNode::new(node("Plant.Reactor.Temperature"), ""),
                OpcuaNode::new("Plant", ""),
            ],
        },
    }));
    let added = events.wait(anonymous, |kind| match kind {
        EventKind::PointsAdded(points) => Some(points.clone()),
        _ => None,
    });
    let names: Vec<&str> = added.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Reactor › Setpoint", "Recipe", "BatchId"]);
    assert_eq!(added.iter().map(|p| p.id).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(added[0].writable && added[1].writable);
    assert!(!added[2].writable, "BatchId is read-only");
    let OpcuaEvent::Monitored(outcomes) = opcua(&mut events, anonymous) else {
        panic!("monitor outcomes");
    };
    let refused: Vec<&str> = outcomes
        .iter()
        .filter(|(_, result)| result.is_err())
        .map(|(id, _)| id.as_str())
        .collect();
    let (nowhere, twice) = (node("Plant.Nowhere"), node("Plant.Reactor.Temperature"));
    assert_eq!(refused, [nowhere.as_str(), twice.as_str(), "Plant"]);
    says(&mut events, anonymous, 2, "Base resin\nGood");

    // Typed writes: a setpoint the plant takes, a read-only variable, and text that is no number.
    assert!(engine.send(Command::Opcua {
        session: anonymous,
        request: OpcuaRequest::Write {
            point: 1,
            text: "120".into(),
        },
    }));
    assert_eq!(written(&mut events, anonymous, 1), Ok(()));
    reads(&mut events, anonymous, 1, |reading| {
        (reading.value - 120.0).abs() < 1e-9
    });
    reads(&mut events, server, 1, |reading| {
        (reading.value - 120.0).abs() < 1e-9
    });
    assert!(engine.send(Command::Opcua {
        session: anonymous,
        request: OpcuaRequest::Write {
            point: 3,
            text: "7".into(),
        },
    }));
    assert!(written(&mut events, anonymous, 3).is_err());
    assert!(engine.send(Command::Opcua {
        session: anonymous,
        request: OpcuaRequest::Write {
            point: 1,
            text: "hot".into(),
        },
    }));
    assert!(written(&mut events, anonymous, 1).is_err());

    // A client writing a measured value holds it in the simulation.
    assert!(engine.send(Command::Write {
        session: anonymous,
        point: 0,
        value: 175.0,
    }));
    assert_eq!(written(&mut events, anonymous, 0), Ok(()));
    events.wait(server, |kind| match kind {
        EventKind::Locked {
            point: 0,
            locked: true,
        } => Some(()),
        _ => None,
    });
    reads(&mut events, anonymous, 0, |reading| {
        (reading.value - 175.0).abs() < 1e-9
    });

    // Methods: their arguments, a call the plant runs, and one it refuses.
    let start_batch = node("Plant.Reactor.StartBatch");
    assert!(engine.send(Command::Opcua {
        session: anonymous,
        request: OpcuaRequest::Method {
            method_id: start_batch.clone(),
        },
    }));
    let OpcuaEvent::Method {
        result: Ok(method), ..
    } = opcua(&mut events, anonymous)
    else {
        panic!("method arguments");
    };
    let inputs: Vec<(&str, &str)> = method
        .inputs
        .iter()
        .map(|a| (a.name.as_str(), a.data_type.as_str()))
        .collect();
    assert_eq!(inputs, [("Recipe", "String"), ("Quantity", "Double")]);
    assert_eq!(method.outputs[0].data_type, "UInt32");
    let call = |arguments: [&str; 2]| Command::Opcua {
        session: anonymous,
        request: OpcuaRequest::Call {
            object_id: node("Plant.Reactor"),
            method_id: start_batch.clone(),
            arguments: arguments.map(str::to_owned).to_vec(),
        },
    };
    assert!(engine.send(call(["Pale ale", "500"])));
    let OpcuaEvent::Called { result, .. } = opcua(&mut events, anonymous) else {
        panic!("call answer");
    };
    assert_eq!(result, Ok(vec!["1002".to_owned()]));
    // The batch number and the recipe may change in one flush or two: wait for both.
    let (mut batch, mut recipe) = (false, false);
    events.wait(anonymous, |kind| {
        match kind {
            EventKind::Values(frame) => {
                batch |= frame
                    .readings
                    .get(3)
                    .is_some_and(|reading| reading.value == 1002.0);
            }
            EventKind::Payloads(texts) => {
                recipe |= texts
                    .iter()
                    .any(|text| text.point == 2 && text.text.starts_with("Pale ale"));
            }
            _ => {}
        }
        (batch && recipe).then_some(())
    });
    assert!(engine.send(call(["Pale ale", "3000"])));
    let OpcuaEvent::Called { result, .. } = opcua(&mut events, anonymous) else {
        panic!("call answer");
    };
    assert!(result.is_err(), "more than the vessel holds");
    assert!(engine.send(call(["Pale ale", "lots"])));
    let OpcuaEvent::Called { result, .. } = opcua(&mut events, anonymous) else {
        panic!("call answer");
    };
    assert!(result.is_err_and(|error| error.starts_with("Quantity")));

    // A user signs in over an encrypted channel; a wrong password does not get in.
    let user = SessionId(3);
    let mut secure = client(
        port,
        vec![OpcuaNode::new(node("Plant.Boiler.SteamPressure"), "")],
    );
    secure.policy = OpcuaSecurityPolicy::Basic256Sha256;
    secure.mode = OpcuaSecurityMode::SignAndEncrypt;
    secure.auth = OpcuaAuth::UserName {
        username: "operator".into(),
        password: "voltrus".into(),
    };
    assert!(engine.send(Command::Open {
        session: user,
        spec: SessionSpec::OpcuaClient(secure.clone()),
    }));
    assert_eq!(state(&mut events, user), SessionState::Connected);
    reads(&mut events, user, 0, |reading| {
        reading.quality == Quality::Good
    });
    let intruder = SessionId(4);
    secure.auth = OpcuaAuth::UserName {
        username: "operator".into(),
        password: "guess".into(),
    };
    assert!(engine.send(Command::Open {
        session: intruder,
        spec: SessionSpec::OpcuaClient(secure),
    }));
    let refused = state(&mut events, intruder);
    assert!(
        matches!(
            refused,
            SessionState::Closed { error: Some(_) } | SessionState::Reconnecting { .. }
        ),
        "{refused:?}"
    );
    assert!(engine.send(Command::Close { session: intruder }));

    // The server's user certificate signs a client in.
    let users = root.join(format!("server-{port}")).join("users");
    let certified = SessionId(5);
    let mut signed = client(port, Vec::new());
    signed.policy = OpcuaSecurityPolicy::Aes256Sha256RsaPss;
    signed.mode = OpcuaSecurityMode::SignAndEncrypt;
    signed.auth = OpcuaAuth::Certificate {
        certificate: users.join("operator.der").display().to_string(),
        private_key: users.join("operator.pem").display().to_string(),
    };
    assert!(engine.send(Command::Open {
        session: certified,
        spec: SessionSpec::OpcuaClient(signed),
    }));
    assert_eq!(state(&mut events, certified), SessionState::Connected);

    // The server goes away: its clients notice and try again.
    assert!(engine.send(Command::Close { session: server }));
    assert!(matches!(
        state(&mut events, server),
        SessionState::Closed { error: None }
    ));
    assert!(matches!(
        state(&mut events, anonymous),
        SessionState::Reconnecting { .. }
    ));
    assert!(engine.send(Command::Shutdown));
}

#[test]
fn a_taken_port_is_a_clear_error() {
    pki_root();
    let (engine, receiver) = vt_engine::start().expect("the engine");
    let mut events = Events::new(receiver);
    let taken = std::net::TcpListener::bind("0.0.0.0:0").expect("a port to hold");
    let port = taken.local_addr().expect("its address").port();
    // Hold the IPv6 side too where the system has it.
    let _v6 = std::net::TcpListener::bind(("::", port));
    let server = SessionId(1);
    assert!(engine.send(Command::Open {
        session: server,
        spec: SessionSpec::OpcuaServer(OpcuaServerSpec {
            port,
            ..OpcuaServerSpec::default()
        }),
    }));
    let closed = state(&mut events, server);
    assert!(
        matches!(&closed, SessionState::Closed { error: Some(error) } if error.contains("in use")),
        "{closed:?}"
    );
    // A server nobody may sign in to is refused before it starts.
    let locked = SessionId(2);
    assert!(engine.send(Command::Open {
        session: locked,
        spec: SessionSpec::OpcuaServer(OpcuaServerSpec {
            port: free_port(),
            anonymous: false,
            username: String::new(),
            password: String::new(),
        }),
    }));
    // The user certificate still lets clients in, so this server starts.
    assert!(matches!(
        state(&mut events, locked),
        SessionState::Listening { .. }
    ));
    assert!(engine.send(Command::Shutdown));
}
