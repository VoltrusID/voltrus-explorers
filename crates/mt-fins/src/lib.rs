//! `mt-fins`: Omron FINS over UDP and FINS/TCP.
//!
//! - [`FinsClient`]: memory area read and write (words and bits) of CIO, W, H, A, DM, EM banks and timer/counter
//!   present values, controller data and status read, RUN/MONITOR and STOP.
//! - [`FinsServer`]: a simulated CJ2-like CPU answering the same commands on one port over UDP and TCP.
//! - [`parse_address`]: addresses as CX-Programmer writes them (`D100`, `CIO0.05`, `W10`, `H0`, `E0_100`).

pub mod area;
pub mod client;
pub mod frame;
pub mod server;

pub use area::{FinsAddress, FinsArea, parse_address};
pub use client::{ControllerData, ControllerStatus, Direction, FinsClient, FinsConfig, FinsError, MAX_BITS, MAX_WORDS, TrafficFn, Transport};
pub use frame::{Command, FinsHeader, FinsResponse, RunMode, end_code_text};
pub use server::{EventFn, FinsMemory, FinsServer, ServerEvent, area_size};

#[cfg(test)]
mod e2e {
    use super::*;
    use std::sync::{Arc, Mutex};

    async fn client(port: u16, transport: Transport) -> FinsClient {
        let mut cfg = FinsConfig::new("127.0.0.1", port);
        cfg.transport = transport;
        cfg.timeout_ms = 2000;
        FinsClient::connect(cfg, None).await.unwrap()
    }

    #[tokio::test]
    async fn udp_and_tcp_read_and_write_every_area() {
        let server = FinsServer::start(0, 10, None).await.unwrap();
        for transport in [Transport::Udp, Transport::Tcp] {
            let c = client(server.port(), transport).await;
            for area in [FinsArea::Dm, FinsArea::Cio, FinsArea::Wr, FinsArea::Hr, FinsArea::Em(2), FinsArea::Tim, FinsArea::Cnt] {
                let at = FinsAddress::new(area, 100);
                c.write_words(at, &[0x1234, 0xFFFF]).await.unwrap();
                assert_eq!(c.read_words(at, 2).await.unwrap(), vec![0x1234, 0xFFFF], "{transport:?} {area:?}");
                c.write_words(at, &[0, 0]).await.unwrap();
            }
            // Bits: CIO0.15 and CIO1.00 in one write crosses a word.
            let at = FinsAddress { area: FinsArea::Cio, word: 0, bit: Some(15) };
            c.write_bits(at, &[true, true]).await.unwrap();
            assert_eq!(c.read_words(FinsAddress::new(FinsArea::Cio, 0), 2).await.unwrap(), vec![0x8000, 0x0001]);
            assert_eq!(c.read_bits(at, 3).await.unwrap(), vec![true, true, false]);
            c.write_bits(at, &[false, false]).await.unwrap();

            let data = c.controller_data().await.unwrap();
            assert_eq!(data.model, "CJ2M-CPU33");
            c.stop().await.unwrap();
            assert_eq!(c.controller_status().await.unwrap().mode, "PROGRAM");
            c.run(RunMode::Monitor).await.unwrap();
            let st = c.controller_status().await.unwrap();
            assert_eq!((st.status.as_str(), st.mode.as_str()), ("Run", "MONITOR"));
            c.disconnect().await;
        }
    }

    #[tokio::test]
    async fn tcp_handshake_assigns_nodes() {
        let server = FinsServer::start(0, 10, None).await.unwrap();
        let c = client(server.port(), Transport::Tcp).await;
        let (sa1, da1) = c.nodes().await.unwrap();
        assert_eq!(da1, 10);
        assert_ne!(sa1, 10);
        assert_ne!(sa1, 0);
    }

    #[tokio::test]
    async fn udp_nodes_default_to_last_octets() {
        let server = FinsServer::start(0, 10, None).await.unwrap();
        let c = client(server.port(), Transport::Udp).await;
        assert_eq!(c.nodes().await.unwrap(), (1, 1));
    }

    #[tokio::test]
    async fn errors_are_end_codes() {
        let server = FinsServer::start(0, 10, None).await.unwrap();
        let c = client(server.port(), Transport::Udp).await;
        let err = c.read_words(FinsAddress::new(FinsArea::Wr, 511), 2).await.unwrap_err();
        assert!(matches!(err, FinsError::EndCode { code: 0x1104 }), "{err}");
        let err = c.write_words(FinsAddress::new(FinsArea::Ar, 10), &[1]).await.unwrap_err();
        assert!(matches!(err, FinsError::EndCode { code: 0x2101 }), "{err}");
        assert!(err.to_string().contains("read-only"), "{err}");
        c.write_words(FinsAddress::new(FinsArea::Ar, 448), &[1]).await.unwrap();
    }

    #[tokio::test]
    async fn large_reads_are_split() {
        let server = FinsServer::start(0, 10, None).await.unwrap();
        server.memory().lock().unwrap().write_words(FinsAddress::new(FinsArea::Dm, 0), &(0..3000u16).collect::<Vec<_>>()).unwrap();
        let c = client(server.port(), Transport::Tcp).await;
        assert_eq!(c.read_words(FinsAddress::new(FinsArea::Dm, 0), 3000).await.unwrap(), (0..3000u16).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn events_and_traffic() {
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = events.clone();
        let server = FinsServer::start(0, 10, Some(Arc::new(move |e: ServerEvent| sink.lock().unwrap().push(e.text)))).await.unwrap();
        let seen: Arc<Mutex<Vec<(Direction, usize)>>> = Arc::default();
        let s2 = seen.clone();
        let cfg = FinsConfig::new("127.0.0.1", server.port());
        let c = FinsClient::connect(cfg, Some(Arc::new(move |d, b: &[u8]| s2.lock().unwrap().push((d, b.len()))))).await.unwrap();
        c.write_words(FinsAddress::new(FinsArea::Dm, 5), &[7]).await.unwrap();
        assert_eq!(*events.lock().unwrap(), vec!["wrote 1 word(s) at D5: [7]".to_string()]);
        assert_eq!(*seen.lock().unwrap(), vec![(Direction::Tx, 20), (Direction::Rx, 14)]);
    }
}
