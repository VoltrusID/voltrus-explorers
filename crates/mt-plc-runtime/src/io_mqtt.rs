use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;

use crate::io_manager::{IoAddress, IoArea, IoDriver, IoError};
use crate::types::PlcValue;
use mt_mqtt::{MqttClientConfig, MqttPubClient};

pub struct MqttIoDriver {
    host: String,
    port: u16,
    client_id: String,
    topic_prefix: String,
    connected: Arc<AtomicBool>,
    cache: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    publish_client: Option<rumqttc::AsyncClient>,
    _event_task: Option<tokio::task::JoinHandle<()>>,
}

impl MqttIoDriver {
    pub fn new(host: String, port: u16, client_id: String, topic_prefix: String) -> Self {
        Self {
            host,
            port,
            client_id,
            topic_prefix,
            connected: Arc::new(AtomicBool::new(false)),
            cache: Arc::new(Mutex::new(HashMap::new())),
            publish_client: None,
            _event_task: None,
        }
    }

    pub async fn connect(&mut self) -> Result<(), IoError> {
        let config = MqttClientConfig {
            host: self.host.clone(),
            port: self.port,
            client_id: self.client_id.clone(),
            username: None,
            password: None,
            ..Default::default()
        };
        let mut client = MqttPubClient::connect(config).await.map_err(|e| IoError {
            message: format!("MQTT connect: {}", e),
            recoverable: true,
        })?;

        let publish_client = client.raw_client().clone();
        let cache = self.cache.clone();
        let connected = self.connected.clone();

        let handle = tokio::spawn(async move {
            connected.store(true, Ordering::SeqCst);
            while let Some(event) = client.next_event().await {
                let msg = match event {
                    mt_mqtt::MqttClientEvent::Message(msg) => msg,
                    _ => continue,
                };
                let mut cache = cache.lock().await;
                cache.insert(msg.topic.clone(), msg.payload.clone());
            }
            connected.store(false, Ordering::SeqCst);
        });

        self.publish_client = Some(publish_client);
        self._event_task = Some(handle);
        Ok(())
    }

    fn io_to_topic(&self, addr: &IoAddress) -> String {
        let area = match addr.area {
            IoArea::Input => "input",
            IoArea::Output => "output",
            IoArea::Memory => "memory",
        };
        let prefix = self.topic_prefix.trim_end_matches('/');
        if prefix.is_empty() {
            format!("plc/{}/{}", area, addr.offset)
        } else {
            format!("{}/{}/{}", prefix, area, addr.offset)
        }
    }

    fn parse_cache(val: &[u8]) -> PlcValue {
        let s = String::from_utf8_lossy(val);
        let trimmed = s.trim();
        if trimmed.eq_ignore_ascii_case("true") || trimmed == "1" {
            PlcValue::Bool(true)
        } else if trimmed.eq_ignore_ascii_case("false") || trimmed == "0" {
            PlcValue::Bool(false)
        } else if let Ok(i) = trimmed.parse::<i32>() {
            PlcValue::Dint(i)
        } else if let Ok(f) = trimmed.parse::<f64>() {
            PlcValue::Real(f as f32)
        } else {
            PlcValue::Dint(0)
        }
    }
}

impl IoDriver for MqttIoDriver {
    fn read_inputs(&mut self, addresses: &[IoAddress]) -> Result<Vec<PlcValue>, IoError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Ok(addresses.iter().map(|_| PlcValue::Dint(0)).collect());
        }
        let rt = tokio::runtime::Handle::current();
        let cache = rt.block_on(async { self.cache.lock().await });
        Ok(addresses
            .iter()
            .map(|a| {
                let topic = self.io_to_topic(a);
                cache
                    .get(&topic)
                    .map(|b| Self::parse_cache(b))
                    .unwrap_or(PlcValue::Dint(0))
            })
            .collect())
    }

    fn write_outputs(&mut self, mappings: &[(IoAddress, PlcValue)]) -> Result<(), IoError> {
        let client = self.publish_client.as_ref().ok_or(IoError {
            message: "MQTT not connected".into(),
            recoverable: true,
        })?;
        let rt = tokio::runtime::Handle::current();
        for (addr, val) in mappings {
            let topic = self.io_to_topic(addr);
            let payload = match val {
                PlcValue::Bool(b) => (if *b { "true" } else { "false" }).as_bytes().to_vec(),
                PlcValue::Int(i) => i.to_string().as_bytes().to_vec(),
                PlcValue::Dint(i) => i.to_string().as_bytes().to_vec(),
                PlcValue::Real(f) => format!("{:.6}", f).as_bytes().to_vec(),
                PlcValue::Word(w) => w.to_string().as_bytes().to_vec(),
                PlcValue::Byte(b) => b.to_string().as_bytes().to_vec(),
                _ => vec![],
            };
            rt.block_on(async {
                client
                    .publish(&topic, rumqttc::QoS::AtLeastOnce, false, payload)
                    .await
            })
            .map_err(|e| IoError {
                message: format!("MQTT publish: {}", e),
                recoverable: true,
            })?;
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "mqtt"
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn disconnect(&mut self) {
        self._event_task.take(); // Drop the handle — the task will stop since next_event returns None on errors
        self.connected.store(false, Ordering::SeqCst);
        self.publish_client = None;
    }
}
