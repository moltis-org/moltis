use {
    moltis_caldav::{ical::EventComponentIndex, types::CalendarInfo},
    moltis_connectors::{ConnectorItemInput, SourceObservation},
    serde::Serialize,
};

#[derive(Debug, Clone, PartialEq)]
pub struct CalDavSnapshot {
    pub items: Vec<ConnectorItemInput>,
    pub source_observations: Vec<SourceObservation>,
    pub calendars: Vec<CalendarInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CalendarBody {
    pub schema_version: u32,
    pub metadata: CalendarInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseStatus {
    Parsed,
    Unparsed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventBody {
    pub schema_version: u32,
    pub calendar_href: String,
    pub resource_href: String,
    pub uid: Option<String>,
    pub components: Vec<EventComponentIndex>,
    pub raw_ical: String,
    pub parse_status: ParseStatus,
}
