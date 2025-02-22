//! WebSocket handshake machine.

use bytes::Buf;
use log::*;
use std::io::{Cursor, Read, Write};

use crate::{
    error::{Error, ProtocolError, Result},
    util::NonBlockingResult,
    ReadBuffer,
};

/// A generic handshake state machine.
#[derive(Debug)]
pub struct HandshakeMachine<Stream> {
    stream: Stream,
    state: HandshakeState,
}

impl<Stream> HandshakeMachine<Stream> {
    /// Start reading data from the peer.
    pub fn start_read(stream: Stream) -> Self {
        HandshakeMachine { stream, state: HandshakeState::Reading(ReadBuffer::new()) }
    }
    /// Start writing data to the peer.
    pub fn start_write<D: Into<Vec<u8>>>(stream: Stream, data: D) -> Self {
        HandshakeMachine { stream, state: HandshakeState::Writing(Cursor::new(data.into())) }
    }
    /// Returns a shared reference to the inner stream.
    pub fn get_ref(&self) -> &Stream {
        &self.stream
    }
    /// Returns a mutable reference to the inner stream.
    pub fn get_mut(&mut self) -> &mut Stream {
        &mut self.stream
    }
}

impl<Stream: Read + Write> HandshakeMachine<Stream> {
    /// Perform a single handshake round.
    pub fn single_round<Obj: TryParse>(mut self) -> Result<RoundResult<Obj, Stream>> {
        trace!("Doing handshake round.");
        match self.state {
            HandshakeState::Reading(mut buf) => {
                let read = buf.read_from(&mut self.stream).no_block()?;
                match read {
                    Some(0) => Err(Error::Protocol(ProtocolError::HandshakeIncomplete)),
                    Some(_) => Ok(if let Some((size, obj)) = Obj::try_parse(Buf::chunk(&buf))? {
                        buf.advance(size);
                        RoundResult::StageFinished(StageResult::DoneReading {
                            result: obj,
                            stream: self.stream,
                            tail: buf.into_vec(),
                        })
                    } else {
                        RoundResult::Incomplete(HandshakeMachine {
                            state: HandshakeState::Reading(buf),
                            ..self
                        })
                    }),
                    None => Ok(RoundResult::WouldBlock(HandshakeMachine {
                        state: HandshakeState::Reading(buf),
                        ..self
                    })),
                }
            }
            HandshakeState::Writing(mut buf) => {
                if self.stream.flush().is_ok() {
                    if buf.has_remaining() {
                        if let Some(size) = self.stream.write(Buf::chunk(&buf)).no_block()? {
                            assert!(size > 0);
                            buf.advance(size);
                            Ok(RoundResult::Incomplete(HandshakeMachine {
                                state: HandshakeState::Writing(buf),
                                ..self
                            }))
                        } else {
                            Ok(RoundResult::WouldBlock(HandshakeMachine {
                                state: HandshakeState::Writing(buf),
                                ..self
                           }))
                        }
                    } else {
                        Ok(RoundResult::StageFinished(StageResult::DoneWriting(self.stream)))
                    }
                } else {
                    Ok(RoundResult::WouldBlock(HandshakeMachine {
                        state: HandshakeState::Writing(buf),
                        ..self
                    }))
                }
            }
        }
    }
}

/// The result of the round.
#[derive(Debug)]
pub enum RoundResult<Obj, Stream> {
    /// Round not done, I/O would block.
    WouldBlock(HandshakeMachine<Stream>),
    /// Round done, state unchanged.
    Incomplete(HandshakeMachine<Stream>),
    /// Stage complete.
    StageFinished(StageResult<Obj, Stream>),
}

/// The result of the stage.
#[derive(Debug)]
pub enum StageResult<Obj, Stream> {
    /// Reading round finished.
    #[allow(missing_docs)]
    DoneReading { result: Obj, stream: Stream, tail: Vec<u8> },
    /// Writing round finished.
    DoneWriting(Stream),
}

/// The parseable object.
pub trait TryParse: Sized {
    /// Return Ok(None) if incomplete, Err on syntax error.
    fn try_parse(data: &[u8]) -> Result<Option<(usize, Self)>>;
}

/// The handshake state.
#[derive(Debug)]
enum HandshakeState {
    /// Reading data from the peer.
    Reading(ReadBuffer),
    /// Sending data to the peer.
    Writing(Cursor<Vec<u8>>),
}
