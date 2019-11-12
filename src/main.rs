use clap::{App, Arg};
use colored::*;
use rayon::prelude::*;
use rayon::*;
use select::document::Document;
use select::node::Node;
use select::predicate::*;
use std::num::ParseIntError;
use std::ops::RangeInclusive;
use std::process;

// Service URLs
const BASE_URL: &str = "http://mobil.bvg.de/";
const SEARCH_URL: &str = "Fahrinfo/bin/stboard.bin/eox?ld=0.1&&rt=0&start=suchen";

// Columns headers
const LINE_COLUMN: &str = "Line";
const DEPARTURE_COLUMN: &str = "Departure";
const DIRECTION_COLUMN: &str = "Direction";
const STATUS_COLUMN: &str = "Status";
const PLATFORM_COLUMN: &str = "Platform";
const NO_DELAY: &str = "±0\'";

#[derive(Debug)]
struct StationSearch {
    name: String,
    link_to_station_overview: String,
}

#[derive(Debug)]
struct DepartureOverview {
    time: String,
    line: String,
    direction: String,
    platform: Option<String>,
    link_to_departure_detail: String,
}

#[derive(Debug)]
struct StationOverview {
    name: String,
    departures: Vec<DepartureOverview>,
}

#[derive(Debug)]
struct StationDetail {
    name: String,
    departures: Vec<DepartureDetail>,
    disruptions: Vec<String>,
}

#[derive(Debug)]
struct DepartureDetail {
    time: String,
    line: String,
    direction: String,
    platform: Option<String>,
    status: String,
    information: Vec<String>,
}

#[derive(Debug)]
struct ParsedInfo<'a> {
    index: usize,
    time_node: Node<'a>,
    line_node: Node<'a>,
    direction_node: Node<'a>,
}

fn main() -> Result<(), reqwest::Error> {
    let fast_args = get_fast_args();
    let station: String = fast_args.clone().map(|a| a.0).unwrap_or_else(|| {
        println!("Which station are you interested in ?");
        read_user_input()
    });
    let stations = get_stations(&station)?;
    if stations.is_empty() {
        println!("No stations found for `{}`", station);
    } else {
        let user_station_choice = if fast_mode_enabled(&fast_args) {
            1 // in --fast mode the first one is selected
        } else {
            println!("Several stations are available, please select the exact location:");
            display_choices(&stations.iter().map(|s| s.name.to_owned()).collect());
            read_user_choice_range(1..=stations.len())
        };
        let picked_station = stations.get(user_station_choice - 1).unwrap(); // safe unwrap because of `read_user_choice_range`
        let station_overview = get_station_overview(picked_station)?;

        let mut available_lines: Vec<String> = station_overview
            .departures
            .iter()
            .map(|d| d.line.clone())
            .collect();
        available_lines.sort();
        available_lines.dedup();

        if available_lines.is_empty() {
            println!("No lines available at this station");
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
                    println!("Several lines are available, please select the line(s) to display:");
                    display_choices(&available_lines);
                    let user_line_choice = read_user_choice_range(1..=available_lines.len());
                    available_lines
                        .get(user_line_choice - 1)
                        .unwrap()
                        .to_string()
                }
            };
            let station_detail = get_station_detail_for_line(station_overview, line.as_str())?;
            display_departures(station_detail);
        }
    }
    Ok(())
}

fn fast_mode_enabled(args: &Option<(String, String)>) -> bool {
    args.is_some()
}

fn get_fast_args() -> Option<(String, String)> {
    let matches = App::new("zbb")
        .version("0.1.0")
        .author("Arnaud Gourlay <arnaud.gourlay@gmail.com>")
        .about("The BVG realtime schedules in your terminal")
        .arg(
            Arg::with_name("fast")
                .help("used to by-pass the interactive mode")
                .long("fast")
                .short("F")
                .takes_value(true)
                .value_names(&["STATION", "LINE"])
                .required(false)
                .min_values(2)
                .max_values(2),
        )
        .get_matches();

    let fast_args: Option<(String, String)> = matches.values_of("fast").map(|mut values| {
        (
            values.next().unwrap().to_string(),
            values.next().unwrap().to_string(),
        )
    });
    fast_args
}

fn get_station_detail_for_line(
    station_overview: StationOverview,
    line: &str,
) -> Result<StationDetail, reqwest::Error> {
    let departures_detail_res: Result<Vec<DepartureDetail>, reqwest::Error> = station_overview
        .departures
        .par_iter()
        .filter(|&d| d.line == line)
        .map(make_departure_detail)
        .collect();
    let departures = departures_detail_res?;
    let mut disruptions: Vec<String> = departures
        .iter()
        .flat_map(|d| d.information.clone())
        .collect();
    disruptions.sort();
    disruptions.dedup();

    let station_detail = StationDetail {
        name: station_overview.name,
        departures,
        disruptions,
    };
    Ok(station_detail)
}

fn make_departure_detail(
    departure_overview: &DepartureOverview,
) -> Result<DepartureDetail, reqwest::Error> {
    let html = request_html(departure_overview.link_to_departure_detail.as_str())?;
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

fn display_choices(choices: &Vec<String>) -> () {
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
        .unwrap();
    column_padding(header_label, max_elem_size, true)
}

fn display_departures(station_detail: StationDetail) -> () {
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
    let station_name = station_detail.name;
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
    std::io::stdin().read_line(&mut line).unwrap();
    line.trim_end().to_string()
}

fn read_user_choice_int() -> Result<usize, ParseIntError> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
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

fn request_html(url: &str) -> Result<Document, reqwest::Error> {
    let resp = reqwest::get(url)?.text()?;
    Ok(Document::from(resp.as_str()))
}

fn get_stations(user_input: &str) -> Result<Vec<StationSearch>, reqwest::Error> {
    let full_url = format!("{}{}&input={}", BASE_URL, SEARCH_URL, user_input);
    let html = request_html(&full_url)?;
    let stations = html
        .find(Class("select").descendant(Name("a")))
        .map(station_from_node)
        .collect();
    Ok(stations)
}

fn sanitize(s: String) -> String {
    s.replace("\n", "").trim().to_string()
}

fn sanitize_text_node(on: Node) -> String {
    sanitize(on.text())
}

fn station_from_node(node: Node) -> StationSearch {
    StationSearch {
        name: sanitize_text_node(node),
        link_to_station_overview: format!("{}{}", BASE_URL, node.attr("href").unwrap()),
    }
}

fn get_station_overview(station: &StationSearch) -> Result<StationOverview, reqwest::Error> {
    let html = request_html(&station.link_to_station_overview)?;
    let name = html
        .find(Attr("id", "ivu_overview_input").descendant(Name("strong")))
        .take(1)
        .next()
        .unwrap()
        .text();
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
                let time_node = blocks.get(0).unwrap();
                let line_node = blocks.get(1).unwrap();
                let direction_node = blocks.get(2).unwrap();
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
                .unwrap()
                .attr("href")
                .unwrap();
            let link_to_departure_detail = format!("{}{}", BASE_URL, line_link);
            let (line, platform) = if full_line.contains('(') {
                let line = full_line.chars().take_while(|c| c != &'(').collect();
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

    let station_overview = StationOverview { name, departures };
    Ok(station_overview)
}
