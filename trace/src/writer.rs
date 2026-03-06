use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use bincode::{config, encode_into_std_write};

use crate::format::{MAGIC, TraceHeader, TraceRecord, VERSION};

/// Writes trace records to a `.nxs` file with an accompanying `.nxs.idx` index.
///
/// File format:
///   [MAGIC 4B][VERSION 2B][header (bincode)][records...](bincode)
///
/// Index format (`.nxs.idx`):
///   Vec of (timestep: u64, byte_offset: u64) pairs, one per timestep boundary.
pub struct TraceWriter {
    writer: BufWriter<File>,
    idx_writer: BufWriter<File>,
    last_indexed_ts: Option<u64>,
    byte_offset: u64,
}

impl TraceWriter {
    pub fn create(path: impl AsRef<Path>, header: &TraceHeader) -> std::io::Result<Self> {
        let path = path.as_ref();
        let mut writer = BufWriter::new(File::create(path)?);
        let idx_path = path.with_extension("nxs.idx");
        let idx_writer = BufWriter::new(File::create(idx_path)?);

        // Write magic and version
        writer.write_all(&MAGIC)?;
        writer.write_all(&VERSION.to_le_bytes())?;

        // Write header
        let cfg = config::standard();
        let header_bytes = bincode::encode_to_vec(header, cfg).map_err(std::io::Error::other)?;
        // Write header length so reader can skip it
        writer.write_all(&(header_bytes.len() as u32).to_le_bytes())?;
        writer.write_all(&header_bytes)?;

        let byte_offset =
            (MAGIC.len() + size_of::<u16>() + size_of::<u32>() + header_bytes.len()) as u64;

        Ok(Self {
            writer,
            idx_writer,
            last_indexed_ts: None,
            byte_offset,
        })
    }

    pub fn write_record(&mut self, record: &TraceRecord) -> std::io::Result<()> {
        // Write index entry at timestep boundaries
        if self.last_indexed_ts.is_none_or(|ts| ts != record.timestep) {
            self.last_indexed_ts = Some(record.timestep);
            self.idx_writer.write_all(&record.timestep.to_le_bytes())?;
            self.idx_writer.write_all(&self.byte_offset.to_le_bytes())?;
        }

        let cfg = config::standard();
        let n =
            encode_into_std_write(record, &mut self.writer, cfg).map_err(std::io::Error::other)?;
        self.byte_offset += n as u64;
        Ok(())
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()?;
        self.idx_writer.flush()
    }
}

impl Drop for TraceWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}
