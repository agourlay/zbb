use colored::*;
use futures_util::stream::{self, StreamExt};
use reqwest::Client;
use select::document::Document;
use select::node::Node;
use select::predicate::*;
use std::num::ParseIntError;
use std::ops::RangeInclusive;
use std::process;

mod args;
mod models;
mod utils;
mod zbb_errors;

use crate::args::get_fast_args;
use crate::models::*;
use crate::utils::*;
use crate::zbb_errors::ZbbError;

// Service URLs
const BASE_URL: &str = "http://mobil.bvg.de/";
const SEARCH_URL: &str = "Fahrinfo/bin/stboard.bin/eox?ld=0.1&rt=0&start=suchen";

// Columns headers
const LINE_COLUMN: &str = "Line";
const DEPARTURE_COLUMN: &str = "Departure";
const DIRECTION_COLUMN: &str = "Direction";
const STATUS_COLUMN: &str = "Status";
const PLATFORM_COLUMN: &str = "Platform";
const NO_DELAY: &str = "±0\'";

#[tokio::main]
async fn main() -> Result<(), ZbbError> {
    let fast_args = get_fast_args();
    let input_station: String = fast_args.clone().map(|a| a.0).unwrap_or_else(|| {
        println!("Which station are you interested in ?");
        read_user_input()
    });
    let client = Client::builder().build()?;
    let stations = get_stations(&client, &input_station).await?;
    if stations.is_empty() {
        println!("No station found for input `{}`", input_station);
    } else {
        let user_station_choice = if fast_mode_enabled(&fast_args) {
            1 // in --fast mode the first one is selected
        } else {
            let stations_len = stations.len();
            println!(
                "{} stations are available for that name, please select one:",
                stations_len
            );
            let names: Vec<String> = stations.iter().map(|s| s.name.to_owned()).collect();
            display_choices(&names);
            read_user_choice_range(1..=stations_len)
        };
        let picked_station = stations
            .get(user_station_choice - 1)
            .expect("impossible because of `read_user_choice_range`");
        let station_overview = get_station_overview(&client, picked_station).await?;

        let mut available_lines: Vec<String> = station_overview
            .departures
            .iter()
            .map(|d| d.line.clone())
            .collect();
        available_lines.sort();
        available_lines.dedup();

        if available_lines.is_empty() {
            println!("No lines available found for {}", station_overview.name);
        } else {
            let line = match fast_args {
                Some((_, fast_line)) => {
                    if available_lines.contains(&fast_line) {
                        fast_line
                    } else {
                        println!(
                            "{} is not among the available lines {:?}",
                            fast_line, available_lines
                        );
                        process::exit(1);
                    }
                }
                None => {
                    println!("Several lines are available, please select the line to display:");
                    display_choices(&available_lines);
                    let user_line_choice = read_user_choice_range(1..=available_lines.len());
                    available_lines
                        .get(user_line_choice - 1)
                        .expect("user_line_choice for station index invalid")
                        .to_string()
                }
            };
            let station_detail =
                get_station_detail_for_line(&client, &station_overview, line.as_str()).await?;
            display_departures(&station_overview.name, station_detail);
        }
    }
    Ok(())
}

fn fast_mode_enabled(args: &Option<(String, String)>) -> bool {
    args.is_some()
}

async fn get_station_detail_for_line(
    client: &Client,
    station_overview: &StationOverview,
    line: &str,
) -> Result<StationDetail, ZbbError> {
    let fetch_futures = station_overview
        .departures
        .iter()
        .filter(|&d| d.line == line)
        .map(|d| fetch_departure_detail(client, d));
    // play nice with the BVG API :)
    let departures_res: Vec<_> = stream::iter(fetch_futures)
        .buffer_unordered(2)
        .collect::<Vec<_>>()
        .await;
    // collect Results
    let departures = departures_res.into_iter().collect::<Result<Vec<_>, _>>()?;
    let mut disruptions: Vec<String> = departures
        .iter()
        .flat_map(|d| d.information.clone())
        .collect();
    disruptions.sort();
    disruptions.dedup();

    let station_detail = StationDetail {
        departures,
        disruptions,
    };
    Ok(station_detail)
}

async fn fetch_departure_detail(
    client: &Client,
    departure_overview: &DepartureOverview,
) -> Result<DepartureDetail, ZbbError> {
    let html = request_html(client, departure_overview.link_to_departure_detail.as_str()).await?;
    let disruptions_nodes: Vec<Node> = html.find(Class("journeyMessageHIM")).collect();
    let information: Vec<String> = disruptions_nodes
        .iter()
        .filter_map(|n| {
            let sanitized = sanitize_text_node(*n);
            if sanitized.is_empty() {
                None
            } else {
                Some(sanitized)
            }
        })
        .collect();

    let query_details = Attr("id", "ivu_trainroute_table")
        .descendant(Name("tr"))
        .descendant(Class("tqTime"));
    let steps_elements_nodes: Vec<Node> = html.find(query_details).collect();
    let delay_minutes = steps_elements_nodes
        .iter()
        .filter_map(|n| {
            let txt = sanitize_text_node(*n);
            if txt.starts_with(&departure_overview.time) && txt.len() > 5 {
                let raw_delay: String = txt.chars().skip(5).take_while(|c| c != &'\\').collect();
                if raw_delay == NO_DELAY {
                    None
                } else {
                    Some(raw_delay)
                }
            } else {
                None
            }
        })
        .next();

    let status = delay_minutes
        .map(|d| format!("delayed {}", d))
        .unwrap_or_else(|| "on time".to_string());
    let departure_detail = DepartureDetail {
        time: departure_overview.time.clone(),
        line: departure_overview.line.clone(),
        direction: departure_overview.direction.clone(),
        platform: departure_overview.platform.clone(),
        status,
        information,
    };
    Ok(departure_detail)
}

fn display_choices(choices: &[String]) {
    let choices_len = choices.len();
    choices.iter().enumerate().for_each(|(index, line)| {
        println!(
            "[{}] {}{}",
            index + 1,
            if choices_len > 9 && index < 9 {
                " "
            } else {
                ""
            },
            line
        )
    });
}

fn column_padding(column_name: &str, max_item_length: usize, header_mode: bool) -> String {
    let column_label_len = column_name.chars().count();
    let padding_size = if max_item_length > column_label_len {
        max_item_length - column_label_len
    } else {
        0
    };
    let extra = if header_mode { 3 } else { 0 }; // for headers we inject some breathing space
    " ".repeat(padding_size + extra)
}

fn padding_for_header<F>(
    departures: &[DepartureDetail],
    field_selector: F,
    header_label: &str,
) -> String
where
    F: Fn(&DepartureDetail) -> &str,
{
    let max_elem_size = departures
        .iter()
        .map(|d| field_selector(d).chars().count())
        .max_by(|x, y| x.cmp(y))
        .expect("expected non empty departures");
    column_padding(header_label, max_elem_size, true)
}

fn display_departures(station_name: &str, station_detail: StationDetail) {
    if !station_detail.disruptions.is_empty() {
        let pretty_disruption: String =
            station_detail
                .disruptions
                .iter()
                .fold(String::new(), |acc, s| {
                    if acc == String::new() {
                        format!("- {}", sentence_chunks(s, 100))
                    } else {
                        format!("{}\n\n- {}", acc, sentence_chunks(s, 100))
                    }
                });
        println!(
            "\n{}\n\n{}\n",
            "* Service disruption *".red().bold().underline(),
            pretty_disruption
        );
    }
    println!("Next departures from {}\n", station_name);
    let departures = station_detail.departures;
    let line_header_padding = padding_for_header(&departures, |d| &d.line, LINE_COLUMN);
    let line_header_len = LINE_COLUMN.len() + line_header_padding.len();

    let departure_header_padding = padding_for_header(&departures, |d| &d.time, DEPARTURE_COLUMN);
    let departure_header_len = DEPARTURE_COLUMN.len() + departure_header_padding.len();

    let status_header_padding = padding_for_header(&departures, |d| &d.status, STATUS_COLUMN);
    let status_header_len = STATUS_COLUMN.len() + status_header_padding.len();

    let direction_header_padding =
        padding_for_header(&departures, |d| &d.direction, DIRECTION_COLUMN);
    let direction_header_len = DIRECTION_COLUMN.len() + direction_header_padding.len();

    let header = format!(
        "{}{}{}{}{}{}{}{}{}",
        LINE_COLUMN,
        line_header_padding,
        DEPARTURE_COLUMN,
        departure_header_padding,
        STATUS_COLUMN,
        status_header_padding,
        DIRECTION_COLUMN,
        direction_header_padding,
        PLATFORM_COLUMN
    );
    let header_len = header.chars().count();

    println!("{}", header.italic());
    println!("{}", "-".repeat(header_len));
    departures.iter().for_each(|d| {
        let after_line_padding = column_padding(&d.line, line_header_len, false);
        let after_departure_padding = column_padding(&d.time, departure_header_len, false);
        let after_status_padding = column_padding(&d.status, status_header_len, false);
        let after_direction_padding = column_padding(&d.direction, direction_header_len, false);
        println!(
            "{}{}{}{}{}{}{}{}{}",
            d.line,
            after_line_padding,
            d.time,
            after_departure_padding,
            d.status,
            after_status_padding,
            d.direction,
            after_direction_padding,
            d.platform.as_ref().unwrap_or(&"".to_string())
        );
    });
    println!("{}", "-".repeat(header_len))
}

fn sentence_chunks(s: &str, max_line_len: usize) -> String {
    if s.chars().count() < max_line_len {
        s.to_string()
    } else {
        let (head, tail) = if s.is_char_boundary(max_line_len) {
            s.split_at(max_line_len)
        } else {
            s.split_at(max_line_len - 1) // YOLO
        };
        if tail.chars().count() > max_line_len {
            format!("{}-\n{}", head, sentence_chunks(tail, max_line_len))
        } else {
            format!("{}-\n{}", head, tail)
        }
    }
}

fn read_user_input() -> String {
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .expect("enable to read line from user");
    line.trim_end().to_string()
}

fn read_user_choice_int() -> Result<usize, ParseIntError> {
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .expect("enable to read choice from user");
    line.trim_end().parse::<usize>()
}

fn read_user_choice_range(r: RangeInclusive<usize>) -> usize {
    match read_user_choice_int() {
        Ok(choice) if r.contains(&choice) => choice,
        _ => {
            println!(
                "Invalid input - please pick a number between {} and {}",
                r.start(),
                r.end()
            );
            read_user_choice_range(r)
        }
    }
}

async fn request_html(client: &Client, url: &str) -> Result<Document, reqwest::Error> {
    let resp = client.get(url).send().await?.text().await?;
    Ok(Document::from(resp.as_str()))
}

async fn get_stations(client: &Client, user_input: &str) -> Result<Vec<StationSearch>, ZbbError> {
    let full_url = format!("{}{}&input={}", BASE_URL, SEARCH_URL, user_input);
    let html = request_html(client, &full_url).await?;
    let stations = html
        .find(Class("select").descendant(Name("a")))
        .map(station_from_node)
        .collect();
    Ok(stations)
}

fn station_from_node(node: Node) -> StationSearch {
    StationSearch {
        name: sanitize_text_node(node),
        link_to_station_overview: format!(
            "{}{}",
            BASE_URL,
            node.attr("href")
                .unwrap_or_else(|| panic!("expected to find an href node {:#?}", node))
        ),
    }
}

async fn get_station_overview(
    client: &Client,
    station: &StationSearch,
) -> Result<StationOverview, ZbbError> {
    let html = request_html(client, &station.link_to_station_overview).await?;
    let query = Class("ivu_table")
        .descendant(Name("tbody"))
        .descendant(Name("tr"));
    let elements: Vec<Node> = html.find(query).collect();
    let parsed_blocks: Vec<ParsedInfo> = elements
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            let blocks: Vec<Node> = node.find(Name("td")).into_selection().iter().collect();
            if blocks.len() == 3 {
                let time_node = blocks.get(0).expect("expected 3 blocks");
                let line_node = blocks.get(1).expect("expected 3 blocks");
                let direction_node = blocks.get(2).expect("expected 3 blocks");
                Some(ParsedInfo {
                    index,
                    time_node: *time_node,
                    line_node: *line_node,
                    direction_node: *direction_node,
                })
            } else {
                // Disruptions are sometimes encoded as additional <tr> instead of an enclosed <td>.
                // They are filtered out and will be fetched on the detail page
                None
            }
        })
        .collect();

    let departures = parsed_blocks
        .iter()
        .map(|pd| {
            let time = sanitize_text_node(pd.time_node);
            let full_line = sanitize_text_node(pd.line_node);
            let line_link = pd
                .line_node
                .find(Name("a"))
                .into_selection()
                .first()
                .and_then(|f| f.attr("href"))
                .expect("expected href");
            let link_to_departure_detail = format!("{}{}", BASE_URL, line_link);
            let (line, platform) = if full_line.contains("platf.") {
                let line = full_line.split("platf.").take(1).collect();
                let platform_raw: String = full_line
                    .chars()
                    .rev()
                    .skip(1)
                    .take_while(|c| c != &'.')
                    .collect();
                let platform_pretty: String = sanitize(platform_raw.chars().rev().collect());
                (line, Some(platform_pretty))
            } else {
                (full_line, None)
            };
            let direction = sanitize_text_node(pd.direction_node);
            DepartureOverview {
                time,
                line,
                direction,
                platform,
                link_to_departure_detail,
            }
        })
        .collect();
    let name = station.name.clone();
    let station_overview = StationOverview { name, departures };
    Ok(station_overview)
}
