use bacnet_rs::object::{ObjectType, PropertyIdentifier};

/// Convert a human-readable string to a BACnet [`ObjectType`].
pub fn object_type_from_str(s: &str) -> Option<ObjectType> {
    match s.to_lowercase().as_str() {
        "analoginput" | "ai" => Some(ObjectType::AnalogInput),
        "analogoutput" | "ao" => Some(ObjectType::AnalogOutput),
        "analogvalue" | "av" => Some(ObjectType::AnalogValue),
        "binaryinput" | "bi" => Some(ObjectType::BinaryInput),
        "binaryoutput" | "bo" => Some(ObjectType::BinaryOutput),
        "binaryvalue" | "bv" => Some(ObjectType::BinaryValue),
        "multistateinput" | "mi" => Some(ObjectType::MultiStateInput),
        "multistateoutput" | "mo" => Some(ObjectType::MultiStateOutput),
        "multistatevalue" | "mv" => Some(ObjectType::MultiStateValue),
        "device" => Some(ObjectType::Device),
        "trendlog" | "trend_log" => Some(ObjectType::TrendLog),
        "trendlogmultiple" | "trend_log_multiple" => Some(ObjectType::TrendLogMultiple),
        "schedule" => Some(ObjectType::Schedule),
        "calendar" => Some(ObjectType::Calendar),
        "notificationclass" | "notification_class" => Some(ObjectType::NotificationClass),
        _ => None,
    }
}

/// Convert a BACnet [`ObjectType`] to a camelCase string.
pub fn object_type_to_string(ot: &ObjectType) -> String {
    match ot {
        ObjectType::AnalogInput => "analogInput".to_string(),
        ObjectType::AnalogOutput => "analogOutput".to_string(),
        ObjectType::AnalogValue => "analogValue".to_string(),
        ObjectType::BinaryInput => "binaryInput".to_string(),
        ObjectType::BinaryOutput => "binaryOutput".to_string(),
        ObjectType::BinaryValue => "binaryValue".to_string(),
        ObjectType::MultiStateInput => "multiStateInput".to_string(),
        ObjectType::MultiStateOutput => "multiStateOutput".to_string(),
        ObjectType::MultiStateValue => "multiStateValue".to_string(),
        ObjectType::Device => "device".to_string(),
        ObjectType::TrendLog => "trendLog".to_string(),
        ObjectType::TrendLogMultiple => "trendLogMultiple".to_string(),
        ObjectType::Schedule => "schedule".to_string(),
        ObjectType::Calendar => "calendar".to_string(),
        ObjectType::NotificationClass => "notificationClass".to_string(),
        _ => format!("{ot:?}"),
    }
}

/// Convert a human-readable string to a BACnet [`PropertyIdentifier`].
/// Falls back to `PresentValue` for unknown names.
pub fn property_id_from_str(s: &str) -> PropertyIdentifier {
    match s.to_lowercase().as_str() {
        "presentvalue" | "present_value" => PropertyIdentifier::PresentValue,
        "description" => PropertyIdentifier::Description,
        "objectname" | "object_name" => PropertyIdentifier::ObjectName,
        "statusflags" | "status_flags" => PropertyIdentifier::StatusFlags,
        "units" => PropertyIdentifier::Units,
        "objectlist" | "object_list" => PropertyIdentifier::ObjectList,
        "objectidentifier" | "object_identifier" => PropertyIdentifier::ObjectIdentifier,
        "objecttype" | "object_type" => PropertyIdentifier::ObjectType,
        "maxapdulengthaccepted" | "max_apdu_length_accepted" => {
            PropertyIdentifier::MaxApduLengthAccepted
        }
        "segmentationsupported" | "segmentation_supported" => {
            PropertyIdentifier::SegmentationSupported
        }
        "vendorname" | "vendor_name" => PropertyIdentifier::VendorName,
        "vendoridentifier" | "vendor_identifier" => PropertyIdentifier::VendorIdentifier,
        "modelname" | "model_name" => PropertyIdentifier::ModelName,
        "firmwarerevision" | "firmware_revision" => PropertyIdentifier::FirmwareRevision,
        "reliability" => PropertyIdentifier::Reliability,
        "outofservice" | "out_of_service" => PropertyIdentifier::OutOfService,
        "statetext" | "state_text" => PropertyIdentifier::StateText,
        "numberofstates" | "number_of_states" => PropertyIdentifier::NumberOfStates,
        "eventstate" | "event_state" => PropertyIdentifier::EventState,
        "highlimit" | "high_limit" => PropertyIdentifier::HighLimit,
        "lowlimit" | "low_limit" => PropertyIdentifier::LowLimit,
        "covincrement" | "cov_increment" => PropertyIdentifier::CovIncrement,
        "priorityarray" | "priority_array" => PropertyIdentifier::PriorityArray,
        "relinquishdefault" | "relinquish_default" => PropertyIdentifier::RelinquishDefault,
        "proportionalconstant" | "proportional_constant" => PropertyIdentifier::ProportionalConstant,
        "integralconstant" | "integral_constant" => PropertyIdentifier::IntegralConstant,
        "derivativeconstant" | "derivative_constant" => PropertyIdentifier::DerivativeConstant,
        "errorlimit" | "error_limit" => PropertyIdentifier::ErrorLimit,
        "setpointreference" | "setpoint_reference" => PropertyIdentifier::SetpointReference,
        "manipulatedvariablereference" => PropertyIdentifier::ManipulatedVariableReference,
        "controlledvariablevalue" => PropertyIdentifier::ControlledVariableValue,
        "controlledvariablereference" => PropertyIdentifier::ControlledVariableReference,
        "protocolversion" | "protocol_version" => PropertyIdentifier::ProtocolVersion,
        "protocolrevision" | "protocol_revision" => PropertyIdentifier::ProtocolRevision,
        "protocolservicessupported" | "protocol_services_supported" => {
            PropertyIdentifier::ProtocolServicesSupported
        }
        "protocolobjecttypessupported" => PropertyIdentifier::ProtocolObjectTypesSupported,
        "systemstatus" | "system_status" => PropertyIdentifier::SystemStatus,
        "localtime" | "local_time" => PropertyIdentifier::LocalTime,
        "localdate" | "local_date" => PropertyIdentifier::LocalDate,
        "databaserevision" | "database_revision" => PropertyIdentifier::DatabaseRevision,
        "activatesafetables" | "activate_safe_tables" => PropertyIdentifier::ActiveCovSubscriptions,
        "logbuffer" | "log_buffer" => PropertyIdentifier::LogBuffer,
        "recordcount" | "record_count" => PropertyIdentifier::RecordCount,
        "totalrecordcount" | "total_record_count" => PropertyIdentifier::TotalRecordCount,
        "loginterval" | "log_interval" => PropertyIdentifier::LogInterval,
        "logdeviceobjectproperty" => PropertyIdentifier::LogDeviceObjectProperty,
        "scheduledefault" | "schedule_default" => PropertyIdentifier::ScheduleDefault,
        "weeklyschedule" | "weekly_schedule" => PropertyIdentifier::WeeklySchedule,
        "effectiverperiod" | "effective_period" => PropertyIdentifier::EffectivePeriod,
        "listofobjectpropertyreferences" => PropertyIdentifier::ListOfObjectPropertyReferences,
        "notificationclass" | "notification_class" => PropertyIdentifier::NotificationClass,
        "eventenable" | "event_enable" => PropertyIdentifier::EventEnable,
        "ackedtransitions" | "acked_transitions" => PropertyIdentifier::AckedTransitions,
        "eventtimestamps" | "event_timestamps" => PropertyIdentifier::EventTimeStamps,
        _ => PropertyIdentifier::PresentValue,
    }
}
