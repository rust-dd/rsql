use super::{ExplainPlan, PlanOutput};
use serde::de::{self, Deserializer, IgnoredAny, SeqAccess, Visitor};
use std::fmt;
use std::io::{self, BufReader, Read};

const CHECKPOINT_BYTES: usize = 8192;

struct CheckedBytes<'a, F> {
    bytes: &'a [u8],
    cancelled: F,
}

impl<F: FnMut() -> bool> Read for CheckedBytes<'_, F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if (self.cancelled)() {
            return Err(io::ErrorKind::ConnectionAborted.into());
        }
        let len = buffer.len().min(CHECKPOINT_BYTES);
        self.bytes.read(&mut buffer[..len])
    }
}

struct FirstPlan;

impl<'de> Visitor<'de> for FirstPlan {
    type Value = ExplainPlan;

    fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("a PostgreSQL JSON EXPLAIN array")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut entries: A) -> Result<Self::Value, A::Error> {
        let plan = entries
            .next_element()?
            .ok_or_else(|| de::Error::custom("Missing EXPLAIN plan"))?;
        while entries.next_element::<IgnoredAny>()?.is_some() {}
        Ok(plan)
    }
}

impl PlanOutput {
    pub fn from_json(raw: String) -> Result<Box<Self>, String> {
        Self::decode(raw, || false).map_err(|e| format!("Invalid EXPLAIN result: {e}"))
    }

    /// Deserializes directly into plan nodes. Reading the next input chunk checks the owner,
    /// including inside a single large JSON string or an ignored extension field.
    pub(crate) fn decode(
        raw: String,
        cancelled: impl FnMut() -> bool,
    ) -> serde_json::Result<Box<Self>> {
        let bytes = CheckedBytes {
            bytes: raw.as_bytes(),
            cancelled,
        };
        let mut decoder = serde_json::Deserializer::from_reader(BufReader::with_capacity(
            CHECKPOINT_BYTES,
            bytes,
        ));
        decoder.disable_recursion_limit();
        let plan = serde_stacker::Deserializer::new(&mut decoder).deserialize_seq(FirstPlan)?;
        decoder.end()?;
        Ok(Box::new(Self { raw, plan }))
    }
}
