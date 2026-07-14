use std::collections::HashMap;

use crate::constants;

/// Represents size limit of the stream to prevent `DoS` attacks.
///
/// Please refer [`Constraints`](crate::Constraints) for more info.
#[derive(Debug)]
#[must_use]
pub struct SizeLimit {
    pub(crate) whole_stream: u64,
    pub(crate) per_field: u64,
    pub(crate) headers: u64,
    pub(crate) field_map: HashMap<String, u64>,
}

impl SizeLimit {
    /// Creates a default size limit which is [`u64::MAX`] for the whole stream
    /// and for each field.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets size limit for the whole stream.
    pub const fn whole_stream(mut self, limit: u64) -> Self {
        self.whole_stream = limit;
        self
    }

    /// Sets size limit for each field.
    pub const fn per_field(mut self, limit: u64) -> Self {
        self.per_field = limit;
        self
    }

    /// Sets size limit for a field's full header block.
    pub const fn headers(mut self, limit: u64) -> Self {
        self.headers = limit;
        self
    }

    /// Sets size limit for a specific field, it overrides the
    /// [`per_field`](Self::per_field) value for this field.
    ///
    /// It is useful when you want to set a size limit on a textual field which
    /// will be stored in memory to avoid potential `DoS` attacks from
    /// attackers running the server out of memory.
    pub fn for_field<N: Into<String>>(mut self, field_name: N, limit: u64) -> Self {
        self.field_map.insert(field_name.into(), limit);
        self
    }

    pub(crate) fn extract_size_limit_for(&self, field: Option<&str>) -> u64 {
        field
            .and_then(|field| self.field_map.get(field))
            .copied()
            .unwrap_or(self.per_field)
    }
}

impl Default for SizeLimit {
    fn default() -> Self {
        Self {
            whole_stream: constants::DEFAULT_WHOLE_STREAM_SIZE_LIMIT,
            per_field: constants::DEFAULT_PER_FIELD_SIZE_LIMIT,
            headers: constants::DEFAULT_HEADERS_SIZE_LIMIT,
            field_map: HashMap::default(),
        }
    }
}
