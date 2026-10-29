//! SHDR adapter: a TCP server that streams the simulated machine's observations in the
//! `timestamp|key|value|...` line format a cppagent adapter connection consumes.

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, watch};

use super::model::SimModel;
use crate::types::{Category, ConditionLevel, Observation, Value};

/// Make a value safe for an SHDR line. Line breaks become spaces. A value containing a pipe
/// has each pipe escaped with a backslash and is wrapped in double quotes, as the SHDR
/// specification requires (`"Text with \| (pipe) character."`).
fn field(s: &str) -> String {
    let s = s.replace(['\r', '\n'], " ");
    if s.contains('|') {
        format!("\"{}\"", s.replace('"', "\\\"").replace('|', "\\|"))
    } else {
        s
    }
}

/// Format one observation as an SHDR line (without the trailing newline).
///
/// - sample/event: `timestamp|key|value`
/// - condition: `timestamp|key|LEVEL|nativeCode|nativeSeverity|qualifier|message`
///
/// Unavailable values are written as `UNAVAILABLE`. `key` is the data item id or name.
pub fn format_observation(obs: &Observation, key: &str) -> String {
    let ts = &obs.timestamp;
    match (&obs.condition, obs.category) {
        (Some(c), _) => {
            let level = match c.level {
                ConditionLevel::Normal => "NORMAL",
                ConditionLevel::Warning => "WARNING",
                ConditionLevel::Fault => "FAULT",
                ConditionLevel::Unavailable => "UNAVAILABLE",
            };
            format!(
                "{ts}|{}|{level}|{}|{}|{}|{}",
                field(key),
                field(c.native_code.as_deref().unwrap_or("")),
                field(c.native_severity.as_deref().unwrap_or("")),
                field(c.qualifier.as_deref().unwrap_or("")),
                field(c.message.as_deref().unwrap_or("")),
            )
        }
        (None, Category::Condition) => format!("{ts}|{}|UNAVAILABLE||||", field(key)),
        (None, _) => {
            let v = match &obs.value {
                Value::Unavailable => "UNAVAILABLE".to_string(),
                other => other.to_text(),
            };
            format!("{ts}|{}|{}", field(key), field(&v))
        }
    }
}

/// The reply to an adapter heartbeat request: `* PING` is answered with `* PONG <ms>`, where
/// `<ms>` is the heartbeat interval the adapter expects.
pub fn pong_for(line: &str, heartbeat_ms: u64) -> Option<String> {
    (line.trim() == "* PING").then(|| format!("* PONG {heartbeat_ms}"))
}

pub(crate) async fn serve(
    listener: TcpListener,
    model: SimModel,
    heartbeat_ms: u64,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((sock, _)) = accepted else { continue };
                let model = model.clone();
                let shutdown = shutdown.clone();
                tokio::spawn(async move {
                    let _ = handle_client(sock, model, heartbeat_ms, shutdown).await;
                });
            }
            _ = shutdown.changed() => return,
        }
    }
}

async fn handle_client(
    sock: TcpStream,
    model: SimModel,
    heartbeat_ms: u64,
    mut shutdown: watch::Receiver<bool>,
) -> std::io::Result<()> {
    let (rd, mut wr) = sock.into_split();
    let mut lines = BufReader::new(rd).lines();
    // Subscribe before the dump so nothing is lost in between (duplicates are harmless).
    let mut events = model.subscribe();
    for obs in model.current_observations() {
        let line = format_observation(&obs, &obs.data_item_id);
        wr.write_all(format!("{line}\n").as_bytes()).await?;
    }

    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(obs) => {
                    let line = format_observation(&obs, &obs.data_item_id);
                    wr.write_all(format!("{line}\n").as_bytes()).await?;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    for obs in model.current_observations() {
                        let line = format_observation(&obs, &obs.data_item_id);
                        wr.write_all(format!("{line}\n").as_bytes()).await?;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return Ok(()),
            },
            line = lines.next_line() => match line? {
                Some(l) => {
                    if let Some(pong) = pong_for(&l, heartbeat_ms) {
                        wr.write_all(format!("{pong}\n").as_bytes()).await?;
                    }
                }
                None => return Ok(()),
            },
            _ = shutdown.changed() => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Condition;

    fn obs(category: Category, value: Value, condition: Option<Condition>) -> Observation {
        Observation {
            data_item_id: "k".into(),
            name: None,
            element: "X".into(),
            category,
            data_item_type: None,
            sub_type: None,
            timestamp: "2026-10-29T02:01:39.506169Z".into(),
            sequence: 1,
            value,
            condition,
            composition_id: None,
            statistic: None,
            duration: None,
            reset_triggered: None,
        }
    }

    #[test]
    fn sample_event_and_unavailable_lines() {
        assert_eq!(
            format_observation(&obs(Category::Sample, Value::Number(12.5), None), "Xpos"),
            "2026-10-29T02:01:39.506169Z|Xpos|12.5"
        );
        assert_eq!(
            format_observation(&obs(Category::Event, Value::Text("ACTIVE".into()), None), "exec"),
            "2026-10-29T02:01:39.506169Z|exec|ACTIVE"
        );
        assert_eq!(
            format_observation(&obs(Category::Sample, Value::Unavailable, None), "Xpos"),
            "2026-10-29T02:01:39.506169Z|Xpos|UNAVAILABLE"
        );
        assert_eq!(
            format_observation(&obs(Category::Event, Value::Text("a|b\nc".into()), None), "msg"),
            "2026-10-29T02:01:39.506169Z|msg|\"a\\|b c\""
        );
    }

    #[test]
    fn condition_lines() {
        let fault = Condition {
            level: ConditionLevel::Fault,
            native_code: Some("E42".into()),
            native_severity: Some("1000".into()),
            qualifier: Some("HIGH".into()),
            message: Some("Servo overload".into()),
            condition_id: None,
        };
        assert_eq!(
            format_observation(&obs(Category::Condition, Value::Text("Servo overload".into()), Some(fault)), "motion_cond"),
            "2026-10-29T02:01:39.506169Z|motion_cond|FAULT|E42|1000|HIGH|Servo overload"
        );
        let normal = Condition {
            level: ConditionLevel::Normal,
            native_code: None,
            native_severity: None,
            qualifier: None,
            message: None,
            condition_id: None,
        };
        assert_eq!(
            format_observation(&obs(Category::Condition, Value::Text(String::new()), Some(normal.clone())), "sys_cond"),
            "2026-10-29T02:01:39.506169Z|sys_cond|NORMAL||||"
        );
        let unavailable = Condition { level: ConditionLevel::Unavailable, ..normal.clone() };
        assert_eq!(
            format_observation(&obs(Category::Condition, Value::Unavailable, Some(unavailable)), "sys_cond"),
            "2026-10-29T02:01:39.506169Z|sys_cond|UNAVAILABLE||||"
        );
    }

    #[test]
    fn ping_pong() {
        assert_eq!(pong_for("* PING", 10000).as_deref(), Some("* PONG 10000"));
        assert_eq!(pong_for("* PING\r", 5000).as_deref(), Some("* PONG 5000"));
        assert_eq!(pong_for("2026|a|b", 10000), None);
    }
}
