use select::node::Node;

pub fn sanitize(s: String) -> String {
    s.replace("\n", "").trim().to_string()
}

pub fn sanitize_text_node(on: Node) -> String {
    sanitize(on.text())
}
