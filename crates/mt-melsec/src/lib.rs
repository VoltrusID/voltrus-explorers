//! `mt-melsec`: Mitsubishi MELSEC communication protocol (MC protocol, called SLMP on newer CPUs).
//!
//! - [`McClient`]: 3E and 4E frames, binary or ASCII, over TCP or UDP. Batch read and write in word and bit units,
//!   CPU model read, remote RUN and STOP. These are the Q-compatible commands, which Q, L, iQ-R and iQ-F (FX5) CPUs
//!   and their Ethernet modules accept.
//! - [`McServer`]: a simulated CPU that answers the same commands over TCP and UDP, detecting the frame and code of
//!   each request.
//! - [`parse_address`]: device addresses as GX Works writes them (`D100`, `M0`, `X1F`, `D100.F`).

pub mod client;
pub mod device;
pub mod frame;
pub mod server;

pub use client::{Direction, MAX_BITS, MAX_WORDS, McClient, McConfig, McError, TrafficFn, Transport, describe};
pub use device::{ALL_DEVICES, McAddress, McDevice, parse_address};
pub use frame::{Command, Encoding, FrameKind, Request, Response, Route, end_code_text};
pub use server::{EventFn, McMemory, McServer, ServerEvent, device_size};

#[cfg(test)]
mod e2e {
    use super::*;
    use std::sync::{Arc, Mutex};

    async fn client(port: u16, transport: Transport, frame: FrameKind, encoding: Encoding) -> McClient {
        let mut cfg = McConfig::new("127.0.0.1", port);
        cfg.transport = transport;
        cfg.frame = frame;
        cfg.encoding = encoding;
        cfg.timeout_ms = 2000;
        McClient::connect(cfg, None).await.unwrap()
    }

    #[tokio::test]
    async fn every_frame_code_and_transport_reads_and_writes() {
        let server = McServer::start(0, None).await.unwrap();
        let port = server.port();
        for transport in [Transport::Tcp, Transport::Udp] {
            for frame in [FrameKind::E3, FrameKind::E4] {
                for encoding in [Encoding::Binary, Encoding::Ascii] {
                    let c = client(port, transport, frame, encoding).await;
                    let what = format!("{transport:?} {frame:?} {encoding:?}");
                    let d = McAddress::new(McDevice::D, 100);
                    c.write_words(d, &[1, 0xBEEF, 300]).await.unwrap();
                    assert_eq!(c.read_words(d, 3).await.unwrap(), vec![1, 0xBEEF, 300], "{what}");

                    let w = McAddress::new(McDevice::W, 0x1A);
                    c.write_words(w, &[0x1234]).await.unwrap();
                    assert_eq!(c.read_words(w, 1).await.unwrap(), vec![0x1234], "{what}");

                    let m = McAddress::new(McDevice::M, 101);
                    c.write_bits(m, &[true, false, true]).await.unwrap();
                    assert_eq!(c.read_bits(m, 3).await.unwrap(), vec![true, false, true], "{what}");
                    // Word access to a bit device sees the same bits: M96..M111, M101 and M103 set.
                    let w96 = c.read_words(McAddress::new(McDevice::M, 96), 1).await.unwrap()[0];
                    assert_eq!(w96, (1 << 5) | (1 << 7), "{what}");

                    let x = McAddress::new(McDevice::X, 0x1F);
                    c.write_bits(x, &[true]).await.unwrap();
                    assert_eq!(c.read_bits(x, 1).await.unwrap(), vec![true], "{what}");
                    c.write_bits(x, &[false]).await.unwrap();
                    c.write_bits(m, &[false, false, false]).await.unwrap();

                    let (model, code) = c.read_cpu_model().await.unwrap();
                    assert_eq!((model.as_str(), code), ("Q03UDVCPU", 0x0366), "{what}");
                    c.disconnect().await;
                }
            }
        }
        server.stop();
    }

    #[tokio::test]
    async fn out_of_range_comes_back_as_an_end_code() {
        let server = McServer::start(0, None).await.unwrap();
        let c = client(server.port(), Transport::Tcp, FrameKind::E3, Encoding::Binary).await;
        let err = c.read_words(McAddress::new(McDevice::D, 32767), 2).await.unwrap_err();
        assert!(matches!(err, McError::EndCode { code: 0xC056 }), "{err}");
        assert!(err.to_string().contains("last device number"), "{err}");
        // Bit units on a word device are refused.
        let err = c.read_bits(McAddress::new(McDevice::D, 0), 1).await.unwrap_err();
        assert!(matches!(err, McError::EndCode { code: 0xC05C }), "{err}");
        // The connection survives an error reply.
        assert_eq!(c.read_words(McAddress::new(McDevice::D, 0), 1).await.unwrap(), vec![0]);
    }

    #[tokio::test]
    async fn large_reads_are_split_into_batches() {
        let server = McServer::start(0, None).await.unwrap();
        let mem = server.memory();
        mem.lock().unwrap().write_words(McAddress::new(McDevice::R, 0), &(0..2000u16).collect::<Vec<_>>()).unwrap();
        let c = client(server.port(), Transport::Udp, FrameKind::E4, Encoding::Binary).await;
        let v = c.read_words(McAddress::new(McDevice::R, 0), 2000).await.unwrap();
        assert_eq!(v, (0..2000u16).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn remote_stop_and_run_and_events() {
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = events.clone();
        let server = McServer::start(0, Some(Arc::new(move |e: ServerEvent| sink.lock().unwrap().push(e.text)))).await.unwrap();
        let c = client(server.port(), Transport::Tcp, FrameKind::E3, Encoding::Ascii).await;
        c.remote_stop().await.unwrap();
        assert!(!server.memory().lock().unwrap().running);
        c.remote_run(false).await.unwrap();
        assert!(server.memory().lock().unwrap().running);
        assert_eq!(*events.lock().unwrap(), vec!["remote STOP".to_string(), "remote RUN".to_string()]);
    }

    #[tokio::test]
    async fn traffic_callback_sees_both_directions() {
        let server = McServer::start(0, None).await.unwrap();
        let seen: Arc<Mutex<Vec<(Direction, usize)>>> = Arc::default();
        let sink = seen.clone();
        let cfg = McConfig::new("127.0.0.1", server.port());
        let c = McClient::connect(cfg, Some(Arc::new(move |d, b: &[u8]| sink.lock().unwrap().push((d, b.len()))))).await.unwrap();
        c.read_words(McAddress::new(McDevice::D, 100), 3).await.unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![(Direction::Tx, 21), (Direction::Rx, 17)]);
    }

    #[tokio::test]
    async fn reconnects_after_the_server_drops_the_connection() {
        let server = McServer::start(0, None).await.unwrap();
        let port = server.port();
        let c = client(port, Transport::Tcp, FrameKind::E3, Encoding::Binary).await;
        c.read_words(McAddress::new(McDevice::D, 0), 1).await.unwrap();
        server.stop();
        drop(server);
        assert!(c.read_words(McAddress::new(McDevice::D, 0), 1).await.is_err());
        let server = McServer::start(port, None).await.unwrap();
        // First call after the drop may still fail on the dead socket; the one after reconnects.
        let mut ok = false;
        for _ in 0..3 {
            if c.read_words(McAddress::new(McDevice::D, 0), 1).await.is_ok() {
                ok = true;
                break;
            }
        }
        assert!(ok, "client did not reconnect");
        server.stop();
    }
}
