/*
 * Copyright 2018 Bitwise IO, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 * -----------------------------------------------------------------------------
 */

//! Message types for PeerMessages

#![allow(unknown_lints, clippy::derive_hash_xor_eq)]

use std::fmt;
use std::hash::{Hash, Hasher};

use protobuf::Message;
use sawtooth_sdk::consensus::engine::{BlockId, PeerMessage};

use crate::error::PbftError;
use crate::hash::verify_sha512;
use crate::protos::pbft_message::{
    PbftMessage, PbftMessageInfo, PbftNewView, PbftSealResponse, PbftSignedVote,
};

/// Wrapper enum for all of the possible PBFT-related messages
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PbftMessageWrapper {
    Message(PbftMessage),
    NewView(PbftNewView),
    SealResponse(PbftSealResponse),
}

/// Container for a received PeerMessage and the PBFT message parsed from it
///
/// The bits of the `PeerMessage` struct that this carries around are used in constructing signed
/// votes.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ParsedMessage {
    /// Serialized ConsensusPeerMessageHeader. Inserted into a signed vote.
    pub header_bytes: Vec<u8>,

    /// Signature for `header_bytes`. Inserted into a signed vote.
    pub header_signature: Vec<u8>,

    /// The parsed PBFT message.
    pub message: PbftMessageWrapper,

    /// The serialized PBFT message. Inserted into a signed vote.
    pub message_bytes: Vec<u8>,

    /// Whether or not this message was self-constructed. Self-constructed messages are skipped
    /// when assembling signed votes, since PBFT doesn't have access to the validator key necessary
    /// to create valid signed messages.
    pub from_self: bool,
}

impl Hash for ParsedMessage {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.message {
            PbftMessageWrapper::Message(m) => m.hash(state),
            PbftMessageWrapper::NewView(m) => m.hash(state),
            PbftMessageWrapper::SealResponse(m) => m.hash(state),
        }
    }
}

impl ParsedMessage {
    /// Constructs a `ParsedMessage` from the given `PbftMessage`.
    ///
    /// Does not add metadata necessary for creating a signed vote from this message.
    pub fn from_pbft_message(message: PbftMessage) -> Self {
        Self {
            from_self: false,
            header_bytes: vec![],
            header_signature: vec![],
            message_bytes: message.write_to_bytes().unwrap(),
            message: PbftMessageWrapper::Message(message),
        }
    }

    /// Constructs a `ParsedMessage` from the given `PbftNewView`.
    ///
    /// Does not add metadata necessary for creating a signed vote from this message.
    pub fn from_new_view_message(message: PbftNewView) -> Self {
        Self {
            from_self: false,
            header_bytes: vec![],
            header_signature: vec![],
            message_bytes: message.write_to_bytes().unwrap(),
            message: PbftMessageWrapper::NewView(message),
        }
    }

    /// Constructs a `ParsedMessage` from the given `PbftSignedVote`.
    ///
    /// Adds metadata necessary for re-creating a signed vote later on.
    pub fn from_signed_vote(vote: &PbftSignedVote) -> Result<Self, PbftError> {
        let message = protobuf::parse_from_bytes(vote.get_message_bytes())
            .map_err(|err| PbftError::SerializationError("Error parsing vote".into(), err))?;

        Ok(Self {
            from_self: false,
            header_bytes: vote.get_header_bytes().to_vec(),
            header_signature: vote.get_header_signature().to_vec(),
            message_bytes: vote.get_message_bytes().to_vec(),
            message: PbftMessageWrapper::Message(message),
        })
    }

    pub fn info(&self) -> &PbftMessageInfo {
        match &self.message {
            PbftMessageWrapper::Message(m) => &m.get_info(),
            PbftMessageWrapper::NewView(m) => &m.get_info(),
            PbftMessageWrapper::SealResponse(m) => &m.get_info(),
        }
    }

    /// Returns the `BlockId` for this message's wrapped `PbftMessage`.
    ///
    /// # Panics
    /// + If the wrapped message is a `NewView` or `SealResponse`, which don't contain a block_id
    pub fn get_block_id(&self) -> BlockId {
        match &self.message {
            PbftMessageWrapper::Message(m) => m.get_block_id().to_vec(),
            PbftMessageWrapper::NewView(_) => {
                panic!("ParsedPeerMessage.get_block_id found a new view message!")
            }
            PbftMessageWrapper::SealResponse(_) => {
                panic!("ParsedPeerMessage.get_block_id found a seal response message!")
            }
        }
    }

    /// Returns the wrapped `PbftNewView`.
    ///
    /// # Panics
    /// + If the wrapped message is a regular message or `SealResponse`, not a `NewView`
    pub fn get_new_view_message(&self) -> &PbftNewView {
        match &self.message {
            PbftMessageWrapper::Message(_) => {
                panic!("ParsedPeerMessage.get_view_change_message found a pbft message!")
            }
            PbftMessageWrapper::NewView(m) => m,
            PbftMessageWrapper::SealResponse(_) => {
                panic!("ParsedPeerMessage.get_view_change_message found a seal response message!")
            }
        }
    }

    /// Returns the wrapped `PbftSealResponse`.
    ///
    /// # Panics
    /// + If the wrapped message is a regular message or `NewView`
    pub fn get_seal_response(&self) -> &PbftSealResponse {
        match &self.message {
            PbftMessageWrapper::Message(_) => {
                panic!("ParsedPeerMessage.get_seal found a pbft message!")
            }
            PbftMessageWrapper::NewView(_) => {
                panic!("ParsedPeerMessage.get_seal found a new view message!")
            }
            PbftMessageWrapper::SealResponse(s) => s,
        }
    }

    /// Constructs a `ParsedMessage` from the given `PeerMessage`.
    ///
    /// Attempts to parse the message contents as a `PbftMessage`, `PbftNewView`, or
    /// `PbftSealResponse` and wraps that in an internal enum.
    pub fn from_peer_message(message: PeerMessage, from_self: bool) -> Result<Self, PbftError> {
        // Self-constructed messages aren't signed, since we don't have access to
        // the validator key necessary for signing them.
        if !from_self {
            verify_sha512(&message.content, &message.header.content_sha512)?;
        }

        let parsed_message = match message.header.message_type.as_str() {
            "SealResponse" => PbftMessageWrapper::SealResponse(
                protobuf::parse_from_bytes::<PbftSealResponse>(&message.content).map_err(
                    |err| {
                        PbftError::SerializationError("Error parsing PbftSealResponse".into(), err)
                    },
                )?,
            ),
            "NewView" => PbftMessageWrapper::NewView(
                protobuf::parse_from_bytes::<PbftNewView>(&message.content).map_err(|err| {
                    PbftError::SerializationError("Error parsing PbftNewView".into(), err)
                })?,
            ),
            _ => PbftMessageWrapper::Message(
                protobuf::parse_from_bytes::<PbftMessage>(&message.content).map_err(|err| {
                    PbftError::SerializationError("Error parsing PbftMessage".into(), err)
                })?,
            ),
        };

        Ok(Self {
            header_bytes: message.header_bytes,
            header_signature: message.header_signature,
            message: parsed_message,
            message_bytes: message.content.clone(),
            from_self,
        })
    }

    /// Constructs a `ParsedMessage` from the given serialized `PbftMessage`
    pub fn from_bytes(message: Vec<u8>, message_type: PbftMessageType) -> Result<Self, PbftError> {
        let mut peer_message = PeerMessage {
            content: message,
            ..Default::default()
        };
        peer_message.header.message_type = String::from(message_type);

        Self::from_peer_message(peer_message, true)
    }
}

// Messages related to PBFT consensus
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub enum PbftMessageType {
    /// Basic message types for the multicast protocol
    PrePrepare,
    Prepare,
    Commit,

    /// Auxiliary PBFT messages
    NewView,
    ViewChange,
    SealRequest,
    SealResponse,

    Unset,
}

impl fmt::Display for PbftMessageType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let txt = match self {
            PbftMessageType::PrePrepare => "PP",
            PbftMessageType::Prepare => "Pr",
            PbftMessageType::Commit => "Co",
            PbftMessageType::NewView => "NV",
            PbftMessageType::ViewChange => "VC",
            PbftMessageType::SealRequest => "Rq",
            PbftMessageType::SealResponse => "Rs",
            PbftMessageType::Unset => "Un",
        };
        write!(f, "{}", txt)
    }
}

impl<'a> From<&'a str> for PbftMessageType {
    fn from(s: &'a str) -> Self {
        match s {
            "PrePrepare" => PbftMessageType::PrePrepare,
            "Prepare" => PbftMessageType::Prepare,
            "Commit" => PbftMessageType::Commit,
            "NewView" => PbftMessageType::NewView,
            "ViewChange" => PbftMessageType::ViewChange,
            "SealRequest" => PbftMessageType::SealRequest,
            "SealResponse" => PbftMessageType::SealResponse,
            _ => {
                warn!("Unhandled PBFT message type: {}", s);
                PbftMessageType::Unset
            }
        }
    }
}

impl From<PbftMessageType> for String {
    fn from(msg_type: PbftMessageType) -> String {
        format!("{:?}", msg_type)
    }
}
