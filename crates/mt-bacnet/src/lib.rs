//! `mt-bacnet` - Standalone BACnet/IP protocol client crate for the MacTools suite.
//!
//! Extracted from the Voltrus SCADA platform, this crate provides:
//! - A [`BacnetClient`] implementing the [`ProtocolClient`](mt_protocol::ProtocolClient) trait
//! - Device discovery via Who-Is / I-Am broadcast ([`discover_devices`])
//! - Object browsing via ReadObjectList ([`browse_objects`])
//! - One-shot read/write helpers ([`read_objects`], [`write_object`])
//!
//! All BACnet-specific types (`DeviceCache`, `AppState`, Axum dependencies) from the
//! original Voltrus codebase have been removed -- this crate is fully standalone.
//!
//! # Example
//!
//! ```no_run
//! use mt_bacnet::{BacnetClient, BacnetConfig, BacnetObject, BacnetValue};
//! use mt_protocol::ProtocolClient;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = BacnetConfig {
//!         id: "ahu-1".into(),
//!         ip: "192.168.1.10".into(),
//!         port: 47808,
//!         device_id: 100,
//!         poll_interval_ms: 1000,
//!     };
//!
//!     let client = BacnetClient::connect(config).await?;
//!     let tags = vec![BacnetObject {
//!         name: "supply_temp".into(),
//!         object_type: "analogInput".into(),
//!         object_instance: 1,
//!         property: "presentValue".into(),
//!     }];
//!     let values = client.read(&tags).await?;
//!     println!("{:?}", values);
//!     client.disconnect().await?;
//!     Ok(())
//! }
//! ```

pub mod alarm;
pub mod client;
pub mod cov;
pub mod device;
pub mod discovery;
pub mod helpers;
pub mod schedule;
pub mod trend;
pub mod transport;
pub mod types;

// Re-export the device simulator API.
pub use device::{SimDevice, SimPointDef, SimSnapPoint, SimSnapshot};

// Re-export the primary public API at the crate root.
pub use alarm::{
    acknowledge_alarm_blocking, get_alarm_summary_blocking, get_event_information_blocking,
    parse_alarm_notification, AlarmNotification, AlarmSummaryEntry, BacnetDate, BacnetTime,
    EventInfoEntry, EventInformationResponse, EventTimeStamp,
};
pub use client::{BacnetClient, read_objects, write_object};
pub use cov::{
    build_cancel_cov_frame, build_subscribe_cov_frame, parse_cov_notification, parse_cov_packet,
    renewal_delay, subscribe_cov_blocking, unsubscribe_cov_blocking, CovNotification, CovPacket,
    CovPropertyValue, CovSubscriptionInfo,
};
pub use discovery::{browse_objects, discover_devices, inspect_device, read_object_properties, units_label, who_is};
pub use schedule::{
    read_schedule_blocking, DaySchedule, ScheduleData, ScheduleEntry,
};
pub use trend::{
    read_trend_log_blocking, read_trend_log_count_blocking, LogRecord, TrendLogData, TrendLogKind,
};
pub use types::{
    Backoff, BacnetConfig, BacnetDiscoveredDevice, BacnetObject, BacnetObjectDetail, BacnetObjectInfo,
    BacnetPropertyEntry, BacnetValue, DeviceInspection, WriteResult,
};
