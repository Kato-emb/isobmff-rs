//! [`MuxOutput`], the output half every mux FSM shares: the framing of the file, and whether the mux FSM still writes

use alloc::vec::Vec;

use isobmff_core::BoxHeader;
use isobmff_sequence::{BoxEvent, BoxWriter, EventBytes};

use crate::Error;

/// Lays the boxes of a file down through its framing, and keeps whether the mux FSM above still writes
///
/// A mux FSM asks [`writing`](Self::writing) before it acts on a call, and
/// hands the result of what it did to [`record`](Self::record), so a failure
/// is kept here, once a call. Where its structure places each box stays with
/// the mux FSM.
///
/// # Contract
///
/// * [`frame`](Self::frame) lays one box down as its header, the pieces of
///   its payload that are not empty, and its end.
/// * [`position`](Self::position) is where the output stands: the end of the
///   bytes the last step was written to, counted from the first byte laid
///   down.
/// * A failure handed to [`record`](Self::record) is reported by every later
///   [`writing`](Self::writing). The bytes made before it are still taken from
///   [`poll_output`](Self::poll_output).
/// * Once [`finish`](Self::finish) succeeds, [`writing`](Self::writing)
///   reports [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished).
#[derive(Debug)]
pub(crate) struct MuxOutput {
    boxes: BoxWriter,
    position: u64,
    state: State,
}

/// Where the output stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Laying the file down as the boxes come
    Writing,
    /// Told the file is over, and taking nothing more
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(Error),
}

impl MuxOutput {
    /// Creates an output waiting at the start of a file
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            boxes: BoxWriter::new(),
            position: 0,
            state: State::Writing,
        }
    }

    /// Returns `Ok` while the mux FSM still takes boxes and samples
    ///
    /// # Errors
    ///
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the file was
    ///   declared over by [`finish`](Self::finish).
    /// * The failure [`record`](Self::record) kept.
    pub(crate) const fn writing(&self) -> Result<(), Error> {
        match self.state {
            State::Writing => Ok(()),
            State::Finished => Err(Error::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Keeps the failure `result` carries, for every call after it, and hands `result` back
    ///
    /// # Errors
    ///
    /// The failure `result` carries.
    pub(crate) const fn record<Value>(
        &mut self,
        result: Result<Value, Error>,
    ) -> Result<Value, Error> {
        if let Err(failure) = &result {
            self.state = State::Failed(*failure);
        }

        result
    }

    /// Returns where the output stands, the end of the bytes the last step was written to
    #[must_use]
    pub(crate) const fn position(&self) -> u64 {
        self.position
    }

    /// Lays one box down as `header`, then each piece of `payload` that is not empty, then its end
    ///
    /// # Errors
    ///
    /// The failures of [`BoxWriter::handle_event`], carried on
    /// [`Sequence`](crate::ErrorKind::Sequence).
    pub(crate) fn frame(
        &mut self,
        header: BoxHeader,
        payload: impl IntoIterator<Item = Vec<u8>>,
    ) -> Result<(), Error> {
        self.lay_down_step(BoxEvent::Header(header))?;
        for piece in payload.into_iter().filter(|piece| !piece.is_empty()) {
            self.lay_down_step(BoxEvent::Payload(piece))?;
        }
        self.lay_down_step(BoxEvent::End)
    }

    /// Hands over the bytes the file has been laid down as so far
    pub(crate) fn poll_output(&mut self) -> Option<EventBytes> {
        self.boxes.poll_output()
    }

    /// Declares the file over
    ///
    /// # Errors
    ///
    /// The failures of [`BoxWriter::finish`], carried on
    /// [`Sequence`](crate::ErrorKind::Sequence).
    pub(crate) fn finish(&mut self) -> Result<(), Error> {
        self.boxes.finish()?;
        self.state = State::Finished;

        Ok(())
    }

    /// Hands one step of the framing over, and moves the position to where its bytes end
    fn lay_down_step(&mut self, step: BoxEvent) -> Result<(), Error> {
        self.position = self.boxes.handle_event(step)?.end;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_core::BoxType;

    use super::{Error, MuxOutput};
    use crate::compact_box_header;

    #[test]
    fn a_failed_output_reports_the_same_failure_for_every_call_after_it() {
        let mut output = MuxOutput::new();
        let failure = Error::box_out_of_order(BoxType::compact(*b"ftyp"));

        assert_eq!(output.record::<()>(Err(failure)), Err(failure));
        assert_eq!(output.writing(), Err(failure));
        assert_eq!(output.writing(), Err(failure));
    }

    #[test]
    fn a_failed_output_hands_over_the_bytes_it_had_already_laid_down() {
        let mut output = MuxOutput::new();
        let header = compact_box_header(BoxType::compact(*b"free"), 4).unwrap();

        output.frame(header, [b"AAAA".to_vec()]).unwrap();
        output
            .record::<()>(Err(Error::box_out_of_order(BoxType::compact(*b"ftyp"))))
            .unwrap_err();

        let mut file = Vec::new();
        while let Some(written) = output.poll_output() {
            file.extend_from_slice(&written);
        }

        assert_eq!(file, b"\0\0\0\x0cfreeAAAA");
    }
}
