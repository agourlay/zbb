use select::node::Node;

#[derive(Debug)]
pub(crate) struct StationSearch {
    pub name: String,
    pub link_to_station_overview: String,
}

#[derive(Debug)]
pub(crate) struct DepartureOverview {
    pub time: String,
    pub line: String,
    pub direction: String,
    pub platform: Option<String>,
    pub link_to_departure_detail: String,
}

#[derive(Debug)]
pub(crate) struct StationOverview {
    pub name: String,
    pub departures: Vec<DepartureOverview>,
}

#[derive(Debug)]
pub(crate) struct StationDetail {
    pub name: String,
    pub departures: Vec<DepartureDetail>,
    pub disruptions: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct DepartureDetail {
    pub time: String,
    pub line: String,
    pub direction: String,
    pub platform: Option<String>,
    pub status: String,
    pub information: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct ParsedInfo<'a> {
    pub index: usize,
    pub time_node: Node<'a>,
    pub line_node: Node<'a>,
    pub direction_node: Node<'a>,
}
