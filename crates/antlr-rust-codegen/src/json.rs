use serde::Serialize;

pub(crate) fn to_pretty_json<T: Serialize>(value: &T) -> String {
    let mut output = serde_json::to_string_pretty(value)
        .expect("JSON model contains only JSON-compatible values");
    output.push('\n');
    output
}
