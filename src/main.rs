#[macro_use]
extern crate lazy_static;

use std::{collections::HashMap, fs};

use actix_web::{get, web, App, HttpResponse, HttpServer};
use ical::parser::ical::component::{IcalCalendar, IcalEvent};
use ics::components::Property;
use ics::{Event, ICalendar};
use serde::Deserialize;
use uuid::Uuid;

const REQUIRED_EVENT_PROPERTIES: [&str; 2] = ["UID", "DTSTAMP"];
const EVENT_PROPERTIES: [&str; 3] = ["DTSTART", "DTEND", "TZID"];
const CONV_EVENT_PROPERTIES: [&str; 3] = ["SUMMARY", "LOCATION", "DESCRIPTION"];

const REQUIRED_CALENDAR_PROPERTIES: [&str; 2] = ["VERSION", "PRODID"];
const CALENDAR_PROPERTIES: [&str; 2] = ["NAME", "DESCRIPTION"];

lazy_static! {
    static ref SUBJECT_NAMES_FILE: String =
        fs::read_to_string("config/subject_names.ron").expect("Failed to read subject names file!");
    static ref SUBJECT_NAMES: HashMap<&'static str, &'static str> =
        ron::from_str::<HashMap<&str, &str>>(SUBJECT_NAMES_FILE.as_str()).unwrap();
    static ref SERVER_PORT: String =
        fs::read_to_string("config/server_port.txt").expect("Failed to read server port file!");
}

fn determine_subject(groups: Vec<&str>) -> Option<String> {
    for group in &groups {
        let mut group_name = group.trim();
        while group_name.chars().last().unwrap().is_digit(10) {
            group_name = &group_name[..group_name.len() - 1];
        }
        for (key, value) in SUBJECT_NAMES.iter() {
            if group_name.contains(key) {
                return Some(value.to_string());
            }
        }
        return Some(group_name.to_string());
    }
    None
}

fn convert_properties<'a>(conv_properties: &'a HashMap<String, String>) -> HashMap<String, String> {
    let mut result = HashMap::<String, String>::new();

    if let Some(summary) = conv_properties.get("SUMMARY") {
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

                    let subject = match determine_subject(groups) {
                        Some(subject) => subject,
                        None => "Onbekend Vak".to_string(),
                    };
                    subject
                } else {
                    summary_input.to_string()
                }
            } else {
                summary_input.to_string()
            }
        };
        result.insert("SUMMARY".to_string(), summary_result);
        result.insert("DESCRIPTION".to_string(), description);
    }

    if let Some(location) = &conv_properties.get("LOCATION") {
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
        result.insert("LOCATION".to_string(), loc_result);
    }

    return result;
}

fn convert_event(event: IcalEvent) -> Event<'static> {
    let mut req_properties = HashMap::<String, String>::new();
    let mut conv_properties = HashMap::<String, String>::new();
    let mut properties = HashMap::<String, String>::new();

    for property in &event.properties {
        if REQUIRED_EVENT_PROPERTIES.contains(&property.name.as_str()) {
            if let Some(value) = &property.value {
                req_properties.insert(property.name.clone(), value.clone());
            }
        } else if CONV_EVENT_PROPERTIES.contains(&property.name.as_str()) {
            if let Some(value) = &property.value {
                conv_properties.insert(property.name.clone(), value.clone());
            }
        } else if EVENT_PROPERTIES.contains(&property.name.as_str()) {
            if let Some(value) = &property.value {
                properties.insert(property.name.clone(), value.clone());
            }
        }
    }

    let dt_stamp = req_properties
        .get("DTSTAMP")
        .expect("Missing DTSTAMP property in event!")
        .clone();
    let original_uuid = req_properties
        .get("UID")
        .expect("Missing UID property in event!")
        .clone();
    let uuid = Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, original_uuid.as_bytes())
        .to_hyphenated()
        .to_string();

    let mut event = Event::new(uuid, dt_stamp);

    for property in properties {
        let (key, value) = property;
        event.push(Property::new(key, value));
    }

    conv_properties = convert_properties(&conv_properties);
    for property in &conv_properties {
        let (key, value) = property;
        event.push(Property::new(key.to_string(), value.to_string()));
    }

    return event;
}

fn convert_calendar(cal_data: IcalCalendar) -> String {
    let mut req_cal_properties = HashMap::<String, String>::new();
    let mut cal_properties = HashMap::<String, String>::new();

    for cal_property in &cal_data.properties {
        if REQUIRED_CALENDAR_PROPERTIES.contains(&cal_property.name.as_str()) {
            req_cal_properties.insert(
                cal_property.name.clone(),
                cal_property
                    .value
                    .clone()
                    .expect("Missing required property value for calendar!"),
            );
        } else if CALENDAR_PROPERTIES.contains(&cal_property.name.as_str()) {
            cal_properties.insert(
                cal_property.name.clone(),
                cal_property
                    .value
                    .clone()
                    .expect("Missing property value for calendar!"),
            );
        }
    }

    let ver = match req_cal_properties.get("VERSION") {
        Some(str) => str,
        None => "2.0",
    };
    let prodid = match req_cal_properties.get("PRODID") {
        Some(str) => str,
        None => "-//calconv",
    };
    let mut calendar = ICalendar::new(ver, prodid);

    for event in cal_data.events {
        let parsed_event = convert_event(event);
        calendar.add_event(parsed_event);
    }

    for property in &cal_properties {
        let (key, value) = property;
        calendar.push(Property::new(key, value));
    }

    calendar.to_string()
}

#[derive(Debug, Deserialize)]
struct ConvRequest {
    c: String,
    url: String,
}

async fn get_request(url: &str) -> Result<String, reqwest::Error> {
    let response = reqwest::get(url).await?;
    let text = response.text().await?;
    Ok(text)
}

async fn conv_somtoday(url: &str) -> Result<String, String> {
    return match get_request(url).await {
        Ok(cal_data) => {
            let reader = ical::IcalParser::new(cal_data.as_bytes());
            for line in reader {
                if let Ok(calendar) = line {
                    let convered_calendar = convert_calendar(calendar.clone());
                    return Ok(convered_calendar);
                }
            }
            Err("Failed to parse calendar!".to_string())
        }
        Err(err) => Err(format!("Error while receiving calendar!/n{}", err)),
    };
}

#[get("/conv")]
async fn parse(info: web::Query<ConvRequest>) -> HttpResponse {
    if info.c == "somtoday" {
        return match conv_somtoday(&info.url).await {
            Ok(str) => HttpResponse::Ok().body(str),
            Err(err) => HttpResponse::InternalServerError().body(err),
        };
    } else {
        return HttpResponse::BadRequest().body("Unknown converter!");
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| App::new().service(parse))
        .bind(SERVER_PORT.clone())?
        .run()
        .await
}
