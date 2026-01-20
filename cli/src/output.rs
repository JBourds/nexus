use csv::Writer;
use runner::ProtocolSummary;
use std::{borrow::Cow, io::Write};

#[derive(Debug, serde::Serialize)]
pub struct ProtocolRecord<'a> {
    node: &'a str,
    protocol: &'a str,
    stdout: Cow<'a, str>,
    stderr: Cow<'a, str>,
}

pub fn to_csv(w: impl Write, summaries: &[ProtocolSummary]) {
    let mut wr = Writer::from_writer(w);
    for summary in summaries {
        wr.serialize(ProtocolRecord {
            node: &summary.node,
            protocol: &summary.protocol,
            stdout: String::from_utf8_lossy(&summary.output.stdout),
            stderr: String::from_utf8_lossy(&summary.output.stderr),
        })
        .expect("couldn't write CSV output");
    }
}
