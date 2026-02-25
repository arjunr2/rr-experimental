//! Implementation of [`Replayer`] that reads from a replay trace (e.g. file, memory, etc.)
//!
//! This is copied almost exactly from the implementation in Wasmtime

use anyhow::Result;
use wasm_crimp::common_events;
use wasm_crimp::{
    RREvent, RecordSettings, ReplayError, ReplayReader, ReplaySettings, Replayer,
    from_replay_reader,
};

/// Buffer to read replay data
pub struct ReplayBuffer {
    /// Reader to read replay trace from
    reader: Box<dyn ReplayReader>,
    /// Settings in replay configuration
    settings: ReplaySettings,
    /// Settings for record configuration (encoded in the trace)
    trace_settings: RecordSettings,
    /// Intermediate static buffer for deserialization
    deser_buffer: Vec<u8>,
    /// Whether buffer has been completely read
    eof_encountered: bool,
}

impl Iterator for ReplayBuffer {
    type Item = Result<RREvent, ReplayError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.eof_encountered {
            return None;
        }
        let ret = 'event_loop: loop {
            let result = from_replay_reader(&mut *self.reader, &mut self.deser_buffer);
            match result {
                Err(e) => {
                    break 'event_loop Some(Err(ReplayError::FailedRead(e)));
                }
                Ok(event) => {
                    if let RREvent::Eof = &event {
                        self.eof_encountered = true;
                        break 'event_loop None;
                    } else if event.is_diagnostic() {
                        continue 'event_loop;
                    } else {
                        log::trace!("Read replay event => {event}");
                        break 'event_loop Some(Ok(event));
                    }
                }
            }
        };
        ret
    }
}

impl Replayer for ReplayBuffer {
    fn new_replayer(reader: impl ReplayReader + 'static, settings: ReplaySettings) -> Result<Self> {
        let mut buf = ReplayBuffer {
            reader: Box::new(reader),
            deser_buffer: vec![0; settings.deserialize_buffer_size],
            settings,
            // This doesn't matter now; will override after reading header
            trace_settings: RecordSettings::default(),
            eof_encountered: false,
        };

        let signature: common_events::TraceSignatureEvent = buf.next_event_typed()?;
        // NOTE: Trace checksum is not needed to be validated here since this replay
        // format is supposed to be indepedent of the Engine.

        // Update the trace settings
        buf.trace_settings = signature.settings;

        if buf.settings.validate && !buf.trace_settings.add_validation {
            log::warn!(
                "Replay validation will be omitted since the recorded trace has no validation metadata..."
            );
        }

        Ok(buf)
    }

    #[inline]
    fn settings(&self) -> &ReplaySettings {
        &self.settings
    }

    #[inline]
    fn trace_settings(&self) -> &RecordSettings {
        &self.trace_settings
    }
}
