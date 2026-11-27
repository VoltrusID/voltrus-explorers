//! A simulated S7 CPU to try clients against: `cargo run -p mt-s7 --example s7sim -- [port]`
//! (default 1102; port 102 needs root). Seeds a few markers and DB1, and moves MW10 and MD20 every
//! second so a watch table has something to show.

use mt_s7::{S7Server, S7Tag, S7Value};

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(1102);
    let tag = |a: &str| S7Tag::parse_address(a).expect("valid address");
    let seed = vec![
        (tag("M0.0"), S7Value::Bool(true)),
        (tag("MW10"), S7Value::Word(0)),
        (tag("MD20"), S7Value::DWord(0f32.to_bits())),
        (tag("DB1.DBW0"), S7Value::Word(250)),
        (tag("DB1.DBD2"), S7Value::DWord(1.5f32.to_bits())),
        (tag("DB1.DBX6.0"), S7Value::Bool(false)),
    ];
    let server = S7Server::start(port, seed).await.expect("listen");
    println!("S7 simulator listening on 127.0.0.1:{} (rack 0, slot 1)", server.port());
    let mut n: u16 = 0;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        n = n.wrapping_add(1);
        let current = (n as f32 * 0.37).sin() * 12.0 + 20.0;
        server
            .apply_unless_written(&[
                (tag("MW10"), S7Value::Word(n)),
                (tag("MD20"), S7Value::DWord(current.to_bits())),
            ])
            .await;
    }
}
