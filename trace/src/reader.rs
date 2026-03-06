use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use bincode::config;

use crate::format::{MAGIC, TraceHeader, TraceRecord, VERSION};

/// Index entry mapping timestep → byte offset in the trace file.
#[derive(Debug, Clone, Copy)]
struct IndexEntry {
    pub timestep: u64,
    pub offset: u64,
}

/// Reads a `.nxs` trace file with optional `.nxs.idx` for O(1) seeking.
pub struct TraceReader {
    reader: BufReader<File>,
    pub header: TraceHeader,
    index: Vec<IndexEntry>,
    data_start: u64,
}

impl std::fmt::Debug for TraceReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceReader")
            .field("header", &self.header)
            .field("index_len", &self.index.len())
            .field("data_start", &self.data_start)
            .finish()
    }
}

impl TraceReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, TraceReadError> {
        let path = path.as_ref();
        let mut file = File::open(path)?;

        // Read and verify magic
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)?;
        if magic != MAGIC {
            return Err(TraceReadError::InvalidMagic);
        }

        // Read and verify version
        let mut ver_bytes = [0u8; 2];
        file.read_exact(&mut ver_bytes)?;
        let version = u16::from_le_bytes(ver_bytes);
        if version != VERSION {
            return Err(TraceReadError::UnsupportedVersion(version));
        }

        // Read header length
        let mut len_bytes = [0u8; 4];
        file.read_exact(&mut len_bytes)?;
        let header_len = u32::from_le_bytes(len_bytes) as usize;

        // Read header
        let mut header_bytes = vec![0u8; header_len];
        file.read_exact(&mut header_bytes)?;
        let cfg = config::standard();
        let (header, _): (TraceHeader, _) =
            bincode::decode_from_slice(&header_bytes, cfg).map_err(TraceReadError::Decode)?;

        let data_start = (MAGIC.len() + size_of::<u16>() + size_of::<u32>() + header_len) as u64;

        // Load index if available
        let idx_path = path.with_extension("nxs.idx");
        let index = if idx_path.exists() {
            Self::load_index(&idx_path)?
        } else {
            Vec::new()
        };

        Ok(Self {
            reader: BufReader::new(file),
            header,
            index,
            data_start,
        })
    }

    fn load_index(path: &Path) -> Result<Vec<IndexEntry>, TraceReadError> {
        let data = std::fs::read(path)?;
        let entry_size = size_of::<u64>() * 2;
        let mut entries = Vec::with_capacity(data.len() / entry_size);
        let mut i = 0;
        let sz = size_of::<u64>();
        while i + entry_size <= data.len() {
            let end = i + sz;
            let timestep = u64::from_le_bytes(data[i..end].try_into().unwrap());
            let offset = u64::from_le_bytes(data[end..end + sz].try_into().unwrap());
            entries.push(IndexEntry { timestep, offset });
            i += entry_size;
        }
        Ok(entries)
    }

    /// Seek to the start of a given timestep using the index. Returns false
    /// if the timestep is not in the index (falls back to sequential read).
    pub fn seek_to_timestep(&mut self, ts: u64) -> Result<bool, TraceReadError> {
        // Binary search for the largest timestep <= ts
        let pos = self.index.partition_point(|e| e.timestep <= ts);
        if pos == 0 {
            // Before all indexed entries, seek to data start
            self.reader.seek(SeekFrom::Start(self.data_start))?;
            return Ok(!self.index.is_empty());
        }
        let entry = &self.index[pos - 1];
        self.reader.seek(SeekFrom::Start(entry.offset))?;
        Ok(true)
    }

    /// Seek back to the beginning of trace data.
    pub fn rewind(&mut self) -> Result<(), TraceReadError> {
        self.reader.seek(SeekFrom::Start(self.data_start))?;
        Ok(())
    }

    /// Read the next record, or None at EOF.
    pub fn next_record(&mut self) -> Result<Option<TraceRecord>, TraceReadError> {
        let cfg = config::standard();
        match bincode::decode_from_reader::<TraceRecord, _, _>(&mut self.reader, cfg) {
            Ok(record) => Ok(Some(record)),
            Err(bincode::error::DecodeError::Io { inner, .. })
                if inner.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                Ok(None)
            }
            Err(e) => Err(TraceReadError::Decode(e)),
        }
    }

    /// Read all records for a specific timestep. Seeks first if index is available.
    pub fn records_for_timestep(&mut self, ts: u64) -> Result<Vec<TraceRecord>, TraceReadError> {
        self.seek_to_timestep(ts)?;
        let mut records = Vec::new();
        loop {
            match self.next_record()? {
                Some(rec) if rec.timestep < ts => continue,
                Some(rec) if rec.timestep == ts => records.push(rec),
                _ => break,
            }
        }
        Ok(records)
    }

    /// Collect all records from start to (and including) the given timestep.
    pub fn records_through_timestep(
        &mut self,
        ts: u64,
    ) -> Result<Vec<TraceRecord>, TraceReadError> {
        self.rewind()?;
        let mut records = Vec::new();
        loop {
            match self.next_record()? {
                Some(rec) if rec.timestep <= ts => records.push(rec),
                Some(_) => break,
                None => break,
            }
        }
        Ok(records)
    }
}

#[derive(Debug)]
pub enum TraceReadError {
    Io(std::io::Error),
    InvalidMagic,
    UnsupportedVersion(u16),
    Decode(bincode::error::DecodeError),
}

impl From<std::io::Error> for TraceReadError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl std::fmt::Display for TraceReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::InvalidMagic => write!(f, "Invalid trace file magic bytes"),
            Self::UnsupportedVersion(v) => write!(f, "Unsupported trace version: {v}"),
            Self::Decode(e) => write!(f, "Decode error: {e}"),
        }
    }
}

impl std::error::Error for TraceReadError {}
