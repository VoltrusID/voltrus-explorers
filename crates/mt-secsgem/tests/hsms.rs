//! HSMS framing, control handshakes and timers, over in-memory pipes (so paused-clock tests are
//! deterministic) and a few loopback TCP connections.

use std::time::Duration;

use mt_secsgem::hsms::{read_frame, write_frame};
use mt_secsgem::{
    Connector, Frame, Header, HsmsConfig, HsmsConnection, HsmsListener, HsmsState, Item,
    RejectReason, S9Error, S9Kind, SType, SecsError, SecsMessage, SelectStatus, Timer,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use tokio::time::Instant;

fn ctl(stype: SType, session: u16, b2: u8, b3: u8, system: u32) -> Frame {
    Frame {
        header: Header::control(stype, session, b2, b3, system),
        text: vec![],
    }
}

fn data(session: u16, msg: &SecsMessage, system: u32) -> Frame {
    Frame::data(session, msg, system).unwrap()
}

async fn rd(p: &mut DuplexStream) -> Frame {
    read_frame(p, Duration::from_secs(60), 1 << 20)
        .await
        .unwrap()
}

async fn wr(p: &mut DuplexStream, f: &Frame) {
    write_frame(p, f).await.unwrap();
}

/// An HSMS connection on one end of a pipe, the raw pipe end on the other.
fn pair(config: HsmsConfig) -> (HsmsConnection, mt_secsgem::hsms::IncomingRx, DuplexStream) {
    let (a, b) = duplex(1 << 16);
    let (conn, rx) = HsmsConnection::start_io(a, config);
    (conn, rx, b)
}

async fn selected_pair(
    config: HsmsConfig,
) -> (HsmsConnection, mt_secsgem::hsms::IncomingRx, DuplexStream) {
    let (conn, rx, mut peer) = pair(config);
    wr(&mut peer, &ctl(SType::SelectReq, 0, 0, 0, 1)).await;
    let rsp = rd(&mut peer).await;
    assert_eq!(rsp.header.byte3, 0);
    (conn, rx, peer)
}

#[tokio::test]
async fn passive_control_handshakes_are_byte_exact() {
    let (conn, _rx, mut peer) = pair(HsmsConfig::default());
    assert_eq!(conn.state(), HsmsState::NotSelected);

    // Select.req -> Select.rsp, status 0
    peer.write_all(&[0, 0, 0, 10, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1])
        .await
        .unwrap();
    let mut rsp = [0u8; 14];
    peer.read_exact(&mut rsp).await.unwrap();
    assert_eq!(rsp, [0, 0, 0, 10, 0, 0, 0, 0, 0, 2, 0, 0, 0, 1]);
    assert_eq!(conn.state(), HsmsState::Selected);

    // a second Select.req: already active
    wr(&mut peer, &ctl(SType::SelectReq, 0, 0, 0, 2)).await;
    let rsp = rd(&mut peer).await;
    assert_eq!(rsp.header.stype, 2);
    assert_eq!(rsp.header.byte3, 1);

    // Linktest.req -> Linktest.rsp on session 0xFFFF
    peer.write_all(&[0, 0, 0, 10, 0xff, 0xff, 0, 0, 0, 5, 0, 0, 0, 3])
        .await
        .unwrap();
    let mut raw = [0u8; 14];
    peer.read_exact(&mut raw).await.unwrap();
    assert_eq!(raw, [0, 0, 0, 10, 0xff, 0xff, 0, 0, 0, 6, 0, 0, 0, 3]);

    // Deselect.req -> Deselect.rsp status 0, then status 1 (not established)
    wr(&mut peer, &ctl(SType::DeselectReq, 0, 0, 0, 4)).await;
    let rsp = rd(&mut peer).await;
    assert_eq!((rsp.header.stype, rsp.header.byte3), (4, 0));
    assert_eq!(conn.state(), HsmsState::NotSelected);
    wr(&mut peer, &ctl(SType::DeselectReq, 0, 0, 0, 5)).await;
    let rsp = rd(&mut peer).await;
    assert_eq!((rsp.header.stype, rsp.header.byte3), (4, 1));

    // select again, then Separate.req closes the connection without a reply
    wr(&mut peer, &ctl(SType::SelectReq, 0, 0, 0, 6)).await;
    assert_eq!(rd(&mut peer).await.header.byte3, 0);
    wr(&mut peer, &ctl(SType::SeparateReq, 0, 0, 0, 7)).await;
    conn.closed().await;
    assert_eq!(conn.state(), HsmsState::Closed);
    assert_eq!(conn.close_reason(), None);
    let mut rest = Vec::new();
    peer.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty());
}

#[tokio::test]
async fn bad_headers_are_rejected() {
    let (conn, mut rx, mut peer) = pair(HsmsConfig::default());

    // data before Select: Reject.req, reason 4, never delivered
    let s1f1 = SecsMessage::request(1, 1, None);
    wr(&mut peer, &data(0, &s1f1, 10)).await;
    let r = rd(&mut peer).await;
    assert_eq!(r.header.stype, 7);
    assert_eq!(r.header.byte3, 4);
    assert_eq!(r.header.system, 10);

    wr(&mut peer, &ctl(SType::SelectReq, 0, 0, 0, 1)).await;
    rd(&mut peer).await;

    // unknown SType
    let mut f = ctl(SType::SelectReq, 0, 0, 0, 11);
    f.header.stype = 8;
    wr(&mut peer, &f).await;
    let r = rd(&mut peer).await;
    assert_eq!(
        (
            r.header.stype,
            r.header.byte2,
            r.header.byte3,
            r.header.system
        ),
        (7, 8, 1, 11)
    );

    // unknown PType
    let mut f = data(0, &s1f1, 12);
    f.header.ptype = 3;
    wr(&mut peer, &f).await;
    let r = rd(&mut peer).await;
    assert_eq!(
        (
            r.header.stype,
            r.header.byte2,
            r.header.byte3,
            r.header.system
        ),
        (7, 3, 2, 12)
    );

    // a reply nobody is waiting for: transaction not open
    wr(&mut peer, &data(0, &SecsMessage::notify(1, 2, None), 13)).await;
    let r = rd(&mut peer).await;
    assert_eq!(
        (r.header.stype, r.header.byte3, r.header.system),
        (7, 3, 13)
    );

    // a valid primary still reaches the owner
    wr(&mut peer, &data(0, &s1f1, 14)).await;
    let inc = rx.recv().await.unwrap();
    assert_eq!((inc.stream(), inc.function(), inc.system()), (1, 1, 14));
    assert!(inc.w_bit());
    assert_eq!(conn.state(), HsmsState::Selected);
}

#[tokio::test]
async fn active_side_selects_and_exchanges_messages() {
    let (a, b) = duplex(1 << 16);
    let (client, _crx) = HsmsConnection::start_io(a, HsmsConfig::default());
    let (server, mut srx) = HsmsConnection::start_io(b, HsmsConfig::default());
    let mut log = client.subscribe_log();

    client.select().await.unwrap();
    assert!(client.is_selected() && server.is_selected());

    let link = server.link();
    tokio::spawn(async move {
        while let Some(inc) = srx.recv().await {
            let m = inc.message.as_ref().unwrap();
            let item = Item::list([Item::ascii("MDLN"), Item::ascii("1.0")]);
            link.reply(&inc, m.reply(Some(item))).unwrap();
        }
    });
    let reply = client
        .request(SecsMessage::request(1, 1, None))
        .await
        .unwrap();
    assert_eq!(reply.to_sml(), r#"S1F2 <L [2] <A "MDLN"> <A "1.0">>."#);
    client.linktest().await.unwrap();

    let lines: Vec<String> = std::iter::from_fn(|| log.try_recv().ok())
        .map(|e| e.line())
        .collect();
    let joined = lines.join("\n");
    assert!(joined.contains("-> [0x00000001] Select.req"), "{joined}");
    assert!(
        joined.contains("<- [0x00000001] Select.rsp status 0"),
        "{joined}"
    );
    assert!(joined.contains("-> [0x00000002] S1F1 W."), "{joined}");
    assert!(joined.contains("<- [0x00000002] S1F2 <L [2]"), "{joined}");
    assert!(
        joined.contains("Linktest.req") && joined.contains("Linktest.rsp"),
        "{joined}"
    );

    // deselect and select again
    client.deselect().await.unwrap();
    assert_eq!(client.state(), HsmsState::NotSelected);
    assert!(matches!(
        client.request(SecsMessage::request(1, 1, None)).await,
        Err(SecsError::NotSelected)
    ));
    client.select().await.unwrap();
    client.close();
    server.closed().await;
}

#[tokio::test]
async fn select_refusals_are_typed() {
    for (status, expected) in [
        (1, SelectStatus::AlreadyActive),
        (2, SelectStatus::NotReady),
        (3, SelectStatus::Exhausted),
        (9, SelectStatus::Other(9)),
    ] {
        let (conn, _rx, mut peer) = pair(HsmsConfig::default());
        let h = tokio::spawn({
            let conn = conn.link();
            async move { conn.select().await }
        });
        let req = rd(&mut peer).await;
        wr(
            &mut peer,
            &ctl(SType::SelectRsp, 0, 0, status, req.header.system),
        )
        .await;
        assert_eq!(h.await.unwrap(), Err(SecsError::SelectRefused(expected)));
        assert_eq!(conn.state(), HsmsState::NotSelected);
    }
}

#[tokio::test]
async fn transaction_errors_are_typed() {
    let (conn, _rx, mut peer) = selected_pair(HsmsConfig::default()).await;
    let req = SecsMessage::request(2, 41, None);

    // S2F0 abort
    let c = conn.link();
    let h = tokio::spawn({
        let req = req.clone();
        async move { c.request(req).await }
    });
    let f = rd(&mut peer).await;
    assert_eq!((f.header.byte2, f.header.byte3), (0x82, 41));
    wr(
        &mut peer,
        &data(0, &SecsMessage::notify(2, 0, None), f.header.system),
    )
    .await;
    assert_eq!(h.await.unwrap(), Err(SecsError::Aborted { stream: 2 }));

    // S9F5 about this transaction: fails at once, not after T3
    let c = conn.link();
    let h = tokio::spawn({
        let req = req.clone();
        async move { c.request(req).await }
    });
    let f = rd(&mut peer).await;
    let s9 = SecsMessage::notify(9, 5, Some(Item::Binary(f.header.to_bytes().to_vec())));
    wr(&mut peer, &data(0, &s9, 999)).await;
    match h.await.unwrap() {
        Err(SecsError::S9(S9Error { kind, mhead })) => {
            assert_eq!(kind, S9Kind::UnrecognizedFunction);
            assert_eq!(mhead, f.header.to_bytes());
        }
        other => panic!("{other:?}"),
    }

    // Reject.req
    let c = conn.link();
    let h = tokio::spawn(async move { c.request(req).await });
    let f = rd(&mut peer).await;
    wr(&mut peer, &ctl(SType::RejectReq, 0, 0, 4, f.header.system)).await;
    assert_eq!(
        h.await.unwrap(),
        Err(SecsError::Rejected {
            reason: RejectReason::EntityNotSelected,
            code: 4
        })
    );
    assert_eq!(conn.state(), HsmsState::Selected);
}

#[tokio::test(start_paused = true)]
async fn t3_reply_timeout_fails_one_call_only() {
    let (conn, _rx, mut peer) = selected_pair(HsmsConfig::default()).await;
    let start = Instant::now();
    let c = conn.link();
    let h = tokio::spawn(async move { c.request(SecsMessage::request(1, 1, None)).await });
    let f = rd(&mut peer).await;
    assert_eq!(f.header.stype, 0);
    assert_eq!(h.await.unwrap(), Err(SecsError::Timeout(Timer::T3)));
    assert_eq!(start.elapsed(), Duration::from_secs(45));
    assert_eq!(conn.state(), HsmsState::Selected);

    // a late reply is rejected as "transaction not open"
    wr(
        &mut peer,
        &data(0, &SecsMessage::notify(1, 2, None), f.header.system),
    )
    .await;
    let r = rd(&mut peer).await;
    assert_eq!((r.header.stype, r.header.byte3), (7, 3));
}

#[tokio::test(start_paused = true)]
async fn t3_is_configurable() {
    let cfg = HsmsConfig {
        t3_ms: 2_000,
        ..Default::default()
    };
    let (conn, _rx, _peer) = selected_pair(cfg).await;
    let start = Instant::now();
    let err = conn
        .request(SecsMessage::request(1, 1, None))
        .await
        .unwrap_err();
    assert_eq!(err.timer(), Some(Timer::T3));
    assert_eq!(start.elapsed(), Duration::from_secs(2));
}

#[tokio::test(start_paused = true)]
async fn t6_control_timeout_closes_the_connection() {
    // Select.req unanswered
    let (conn, _rx, mut peer) = pair(HsmsConfig::default());
    let start = Instant::now();
    let c = conn.link();
    let h = tokio::spawn(async move { c.select().await });
    assert_eq!(rd(&mut peer).await.header.stype, 1);
    assert_eq!(h.await.unwrap(), Err(SecsError::Timeout(Timer::T6)));
    assert_eq!(start.elapsed(), Duration::from_secs(5));
    assert_eq!(conn.state(), HsmsState::Closed);
    assert_eq!(conn.close_reason(), Some(SecsError::Timeout(Timer::T6)));

    // Linktest.req unanswered
    let (conn, _rx, _peer) = selected_pair(HsmsConfig::default()).await;
    let err = conn.linktest().await.unwrap_err();
    assert_eq!(err, SecsError::Timeout(Timer::T6));
    assert_eq!(conn.state(), HsmsState::Closed);
    assert!(matches!(
        conn.request(SecsMessage::request(1, 1, None)).await,
        Err(SecsError::NotConnected)
    ));
}

#[tokio::test(start_paused = true)]
async fn t7_closes_a_connection_that_never_selects() {
    let (conn, _rx, _peer) = pair(HsmsConfig::default());
    let start = Instant::now();
    conn.closed().await;
    assert_eq!(start.elapsed(), Duration::from_secs(10));
    assert_eq!(conn.close_reason(), Some(SecsError::Timeout(Timer::T7)));
}

#[tokio::test(start_paused = true)]
async fn t7_restarts_after_deselect_and_stops_on_select() {
    let (conn, _rx, mut peer) = selected_pair(HsmsConfig::default()).await;
    // a selected session may idle forever
    tokio::time::sleep(Duration::from_secs(3600)).await;
    assert_eq!(conn.state(), HsmsState::Selected);
    // after Deselect the not-selected timer runs again
    wr(&mut peer, &ctl(SType::DeselectReq, 0, 0, 0, 2)).await;
    assert_eq!(rd(&mut peer).await.header.byte3, 0);
    let start = Instant::now();
    conn.closed().await;
    assert_eq!(start.elapsed(), Duration::from_secs(10));
    assert_eq!(conn.close_reason(), Some(SecsError::Timeout(Timer::T7)));
}

#[tokio::test(start_paused = true)]
async fn t8_inter_character_timeout() {
    let (conn, _rx, mut peer) = selected_pair(HsmsConfig::default()).await;
    // an idle line is not a T8 timeout, however long
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(conn.state(), HsmsState::Selected);

    // half a length prefix, then silence
    peer.write_all(&[0, 0]).await.unwrap();
    let start = Instant::now();
    conn.closed().await;
    assert_eq!(start.elapsed(), Duration::from_secs(5));
    assert_eq!(conn.close_reason(), Some(SecsError::Timeout(Timer::T8)));
}

#[tokio::test(start_paused = true)]
async fn t8_applies_inside_the_body_and_slow_senders_are_fine() {
    let cfg = HsmsConfig {
        t8_ms: 1_000,
        ..Default::default()
    };
    // a sender that dribbles one byte every 900 ms stays under T8
    let (conn, mut rx, mut peer) = selected_pair(cfg.clone()).await;
    let bytes = data(0, &SecsMessage::request(1, 1, None), 5).encode();
    for b in &bytes {
        peer.write_all(&[*b]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(900)).await;
    }
    assert_eq!(rx.recv().await.unwrap().system(), 5);
    assert_eq!(conn.state(), HsmsState::Selected);

    // a sender that stops in the middle of the text does not
    peer.write_all(&bytes[..12]).await.unwrap();
    conn.closed().await;
    assert_eq!(conn.close_reason(), Some(SecsError::Timeout(Timer::T8)));
}

#[tokio::test(start_paused = true)]
async fn periodic_linktest() {
    let cfg = HsmsConfig {
        linktest_interval_ms: Some(30_000),
        ..Default::default()
    };
    let (conn, _rx, mut peer) = selected_pair(cfg).await;
    let start = Instant::now();
    for n in 0..3u32 {
        let req = rd(&mut peer).await;
        assert_eq!(req.header.stype, 5, "linktest {n}");
        assert_eq!(req.header.session_id, 0xffff);
        assert_eq!(start.elapsed(), Duration::from_secs(30 * u64::from(n + 1)));
        wr(
            &mut peer,
            &ctl(SType::LinktestRsp, 0xffff, 0, 0, req.header.system),
        )
        .await;
    }
    assert_eq!(conn.state(), HsmsState::Selected);
    // an unanswered one ends the session after T6
    let req = rd(&mut peer).await;
    assert_eq!(req.header.stype, 5);
    conn.closed().await;
    assert_eq!(conn.close_reason(), Some(SecsError::Timeout(Timer::T6)));
}

#[tokio::test]
async fn framing_handles_split_and_coalesced_frames() {
    let (conn, mut rx, mut peer) = selected_pair(HsmsConfig::default()).await;
    let a = data(0, &SecsMessage::request(1, 1, None), 100).encode();
    let b = data(
        0,
        &SecsMessage::request(1, 13, Some(Item::list([Item::ascii("x")]))),
        101,
    )
    .encode();

    // two frames in one write
    let mut both = a.clone();
    both.extend_from_slice(&b);
    peer.write_all(&both).await.unwrap();
    assert_eq!(rx.recv().await.unwrap().system(), 100);
    assert_eq!(rx.recv().await.unwrap().system(), 101);

    // one frame in single-byte writes
    for byte in &b {
        peer.write_all(&[*byte]).await.unwrap();
        tokio::task::yield_now().await;
    }
    let inc = rx.recv().await.unwrap();
    assert_eq!(inc.message.unwrap().to_sml(), r#"S1F13 W <L [1] <A "x">>."#);
    assert_eq!(conn.state(), HsmsState::Selected);
}

#[tokio::test]
async fn invalid_lengths_close_the_connection() {
    for bad in [[0u8, 0, 0, 9], [0, 0, 0, 0], [0x7f, 0xff, 0xff, 0xff]] {
        let (conn, _rx, mut peer) = selected_pair(HsmsConfig::default()).await;
        peer.write_all(&bad).await.unwrap();
        conn.closed().await;
        assert!(
            matches!(conn.close_reason(), Some(SecsError::Protocol(_))),
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn undecodable_text_reaches_the_owner_as_an_error() {
    let (_conn, mut rx, mut peer) = selected_pair(HsmsConfig::default()).await;
    let mut f = data(0, &SecsMessage::request(1, 3, None), 7);
    f.text = vec![0x41, 0x05, b'a'];
    wr(&mut peer, &f).await;
    let inc = rx.recv().await.unwrap();
    assert!(inc.message.is_err());
    assert_eq!(inc.system(), 7);
}

#[tokio::test]
async fn dropping_the_last_handle_separates() {
    let (conn, _rx, mut peer) = selected_pair(HsmsConfig::default()).await;
    drop(conn);
    let f = rd(&mut peer).await;
    assert_eq!(f.header.stype, 9);
    let mut rest = Vec::new();
    peer.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty());
}

#[tokio::test]
async fn tcp_listener_and_active_connect() {
    let listener = HsmsListener::bind(("127.0.0.1", 0), HsmsConfig::default())
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (conn, mut rx, _peer) = listener.accept().await.unwrap();
        let inc = rx.recv().await.unwrap();
        conn.reply(&inc, inc.message.as_ref().unwrap().reply(Some(Item::u1(7))))
            .unwrap();
        conn.closed().await;
    });
    let (client, _rx) = HsmsConnection::connect(("127.0.0.1", port), HsmsConfig::default())
        .await
        .unwrap();
    assert!(client.peer_addr().is_some());
    let r = client
        .request(SecsMessage::request(1, 1, None))
        .await
        .unwrap();
    assert_eq!(r.item, Some(Item::u1(7)));
    client.close();
    server.await.unwrap();
}

#[tokio::test]
async fn connect_failure_is_typed() {
    // a port nothing listens on
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let err = HsmsConnection::connect(("127.0.0.1", port), HsmsConfig::default())
        .await
        .err()
        .unwrap();
    assert!(matches!(err, SecsError::ConnectionFailed(_)), "{err:?}");
}

#[tokio::test]
async fn t5_spaces_connection_attempts() {
    let listener = HsmsListener::bind(("127.0.0.1", 0), HsmsConfig::default())
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut keep = Vec::new();
        while let Ok((conn, rx, _)) = listener.accept().await {
            keep.push((conn, rx));
        }
    });
    let cfg = HsmsConfig {
        t5_ms: 400,
        ..Default::default()
    };
    let connector = Connector::new(cfg);
    let start = std::time::Instant::now();
    let (_c1, _r1) = connector.connect(("127.0.0.1", port)).await.unwrap();
    assert!(start.elapsed() < Duration::from_millis(300));
    let (_c2, _r2) = connector.connect(("127.0.0.1", port)).await.unwrap();
    assert!(
        start.elapsed() >= Duration::from_millis(400),
        "{:?}",
        start.elapsed()
    );
    let (_c3, _r3) = connector.connect(("127.0.0.1", port)).await.unwrap();
    assert!(
        start.elapsed() >= Duration::from_millis(800),
        "{:?}",
        start.elapsed()
    );
}
