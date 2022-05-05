use actix_web::body::BoxBody;
use actix_web::error::PayloadError;
use actix_web::web::{Data, Query};
use actix_web::{error, rt};
use actix_web::{get, http::StatusCode, App, HttpResponse, HttpServer, ResponseError};
use awc::Client;
use awc::error::SendRequestError;
use clap::Parser;
use ical::parser::ical::component::IcalCalendar;
use ical::property::Property as SourceProperty;
use ics::components::Property as DestProperty;
use ics::{Event, ICalendar};
use log::{error, LevelFilter};
use serde::Deserialize;
use std::collections::BTreeMap;
use thiserror::Error;
use uuid::Uuid;

#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short)]
    address: String,

    #[clap(short)]
    subjects: String,
}

#[derive(Debug, Clone)]
struct ConvData {
    subject_names: BTreeMap<String, String>,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::builder()
        .format_module_path(false)
        .format_target(false)
        .filter_level(LevelFilter::Info)
        .init();
    let args = Args::parse();
    let subject_names = deserialize_map_arg(args.subjects.clone());
    let address = args.address.clone();

    HttpServer::new(move || {
        App::new().service(convert).app_data(Data::new(ConvData {
            subject_names: subject_names.clone(),
        }))
    })
    .bind(address)?
    .run()
    .await
}

#[derive(Debug, Deserialize)]
struct ConvRequest {
    c: String,
    url: String,
}

#[derive(Debug, Error)]
enum WebError {
    #[error("converter '{0}' not found")]
    ConverterNotFound(String),
    #[error("failed to fetch calendar: {0}")]
    Request(#[from] SendRequestError),
    #[error("failed to fetch calendar: {0}")]
    Payload(#[from] PayloadError),
    #[error("no calendar found")]
    CalendarNotFound,
    #[error("ICAL parsing error: {0}")]
    ICALParseError(#[from] ical::parser::ParserError),
    #[error("conversion error: {0}")]
    Conversion(#[from] ConversionError),
    #[error("blocking error: {0}")]
    BlockingError(#[from] error::BlockingError),
}

impl ResponseError for WebError {
    fn status_code(&self) -> StatusCode {
        match *self {
            WebError::ConverterNotFound(_) => StatusCode::NOT_FOUND,
            WebError::Request(_) => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse<BoxBody> {
        let status_code = self.status_code();
        if !status_code.is_server_error() {
            HttpResponse::build(status_code).body(self.to_string())
        } else {
            error!("server error: {}", self.to_string());
            HttpResponse::build(status_code).finish()
        }
    }
}

#[get("/conv")]
async fn convert(
    query: Query<ConvRequest>,
    data: Data<ConvData>,
) -> Result<HttpResponse, WebError> {
    if query.c == "somtoday" {
        let handle = rt::spawn(async move {
            let calendar = fetch_calendar(&query.url).await?;
            let converted = convert_somtoday(calendar, &data.subject_names)?;
            return Ok::<String, WebError>(converted);
        });
        let result = handle.await.unwrap()?;
        Ok(HttpResponse::Ok().body(result))
    } else {
        Err(WebError::ConverterNotFound(query.c.to_string()))
    }
}

// Deserializes a map defined like this "keya:valuea;keyb;valueb"
fn deserialize_map_arg(input: String) -> BTreeMap<String, String> {
    let mut result = BTreeMap::<String, String>::new();
    for entry in input.split(';') {
        let sep = entry.find(':').expect("Missing ':' seperator!");
        result.insert(entry[0..sep].to_string(), entry[sep..].to_string());
    }
    result
}

async fn fetch_calendar(calendar_url: &str) -> Result<IcalCalendar, WebError> {
    let calendar_str = get_request(calendar_url).await?;
    let mut reader = ical::IcalParser::new(calendar_str.as_bytes());
    // Parse first calendar
    let calendar = reader.next().ok_or(WebError::CalendarNotFound)??;
    Ok(calendar)
}

async fn get_request(url: &str) -> Result<String, WebError> {
    let client = Client::default();
    let mut response = client
        .get(url)
        .insert_header(("User-Agent", "calconv"))
        .send()
        .await?;
    let body = response.body().await?;
    let text = String::from_utf8_lossy(&body).to_string();
    Ok(text)
}

#[derive(Debug, Error)]
enum ConversionError {
    #[error("missing required property: {0}")]
    MissingRequiredProperty(String),
    #[error("missing value of property: {0}")]
    MissingPropertyValue(String),
}

fn extract_properties(
    source: Vec<SourceProperty>,
) -> Result<BTreeMap<String, String>, ConversionError> {
    let mut result = BTreeMap::<String, String>::new();
    for proptery in source.iter() {
        result.insert(
            proptery.name.clone(),
            proptery
                .value
                .clone()
                .ok_or(ConversionError::MissingPropertyValue(proptery.name.clone()))?,
        );
    }
    Ok(result)
}

fn convert_somtoday(
    source: IcalCalendar,
    subject_names: &BTreeMap<String, String>,
) -> Result<String, ConversionError> {
    // Convert properties to string map
    let properties = extract_properties(source.properties)?;

    // Create calendar and add properties
    let mut calendar = ICalendar::new("2.0", "-//rijkvp //calconv //NL");
    for (key, value) in properties.iter() {
        calendar.push(DestProperty::new(key, value));
    }

    // Convert events
    for event in source.events {
        let mut event_properties = extract_properties(event.properties)?;

        // Create event with timestamp & uid
        let dt_stamp =
            properties
                .get("DTSTAMP")
                .ok_or(ConversionError::MissingRequiredProperty(
                    "DTSTAMP".to_string(),
                ))?;
        let original_uuid = properties
            .get("UID")
            .ok_or(ConversionError::MissingRequiredProperty("UID".to_string()))?;
        let uuid = Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, original_uuid.as_bytes())
            .as_hyphenated()
            .to_string();
        let mut event = Event::new(uuid, dt_stamp);

        // Peform actual changes on the properties
        convert_somtoday_properties(&mut event_properties, subject_names)?;

        // Add properties
        for (key, value) in event_properties {
            event.push(DestProperty::new(key, value));
        }
        calendar.add_event(event);
    }

    Ok(calendar.to_string())
}

fn convert_somtoday_properties(
    properties: &mut BTreeMap<String, String>,
    subject_names: &BTreeMap<String, String>,
) -> Result<(), ConversionError> {
    if let Some(summary) = properties.get("SUMMARY") {
        let summary_input = summary.trim().replace("\\", "");
        let mut description = String::new();
        let summary_result: String = {
            if summary_input.contains('-') {
                let parts: Vec<&str> = summary_input.split('-').collect();
                if parts.len() == 3 {
                    let groups_str = parts[1].trim();
                    let groups: Vec<&str> = groups_str.split(',').collect();
                    let teachers_str = parts[2].trim();
                    let teachers: Vec<&str> = groups_str.split(',').collect();

                    if teachers.len() == 1 {
                        description += &format!("Docent: {}; ", teachers_str);
                    } else {
                        description += &format!("Docenten: {}; ", teachers_str);
                    }
                    if groups.len() == 1 {
                        description += &format!("Clustergroep: {}", groups_str);
                    } else {
                        description += &format!("Clustergroepen: {}", groups_str);
                    }

                    let subject = match get_subject(groups, subject_names) {
                        Some(subject) => subject,
                        None => "Onbekend vak".to_string(),
                    };
                    subject
                } else {
                    summary_input.to_string()
                }
            } else {
                summary_input.to_string()
            }
        };
        // Update/create properties
        properties.insert("SUMMARY".to_string(), summary_result);
        properties.insert("DESCRIPTION".to_string(), description);
    }

    if let Some(location) = &properties.get("LOCATION") {
        let loc_input = location.trim().replace("\\", "");
        let locations: Vec<&str> = loc_input.split(',').collect();
        let mut location_list = Vec::new();
        for loc in locations {
            let mut loc = loc.trim();
            if loc.len() == 5 {
                loc = &loc[1..];
            }
            location_list.push(loc.to_uppercase());
        }
        let mut loc_result = String::new();
        let mut first = true;
        for loc in location_list {
            if !first {
                loc_result += ", ";
            } else {
                first = false;
            }
            loc_result += &loc;
        }
        properties.insert("LOCATION".to_string(), loc_result);
    }
    Ok(())
}

fn get_subject(groups: Vec<&str>, subject_names: &BTreeMap<String, String>) -> Option<String> {
    for group in &groups {
        let mut group_name = group.trim();
        while group_name.chars().last().unwrap().is_numeric() {
            group_name = &group_name[..group_name.len() - 1];
        }
        for (key, value) in subject_names.iter() {
            if group_name.contains(key) {
                return Some(value.to_string());
            }
        }
        return Some(group_name.to_string());
    }
    None
}
