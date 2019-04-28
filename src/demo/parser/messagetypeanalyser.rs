use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};

use crate::{ParserState, ReadResult, Stream};
use crate::demo::gameevent_gen::{
    GameEvent, PlayerDeathEvent, PlayerSpawnEvent, TeamPlayRoundWinEvent,
};
use crate::demo::message::{Message, MessageType};
use crate::demo::message::packetentities::EntityId;
use crate::demo::message::usermessage::{ChatMessageKind, SayText2Message, UserMessage};
use crate::demo::packet::PacketType;
use crate::demo::packet::stringtable::StringTableEntry;
use crate::demo::parser::handler::MessageHandler;
use crate::demo::vector::Vector;

pub struct MessageTypeAnalyser {
    packet_types: Vec<MessageType>
}

impl MessageHandler for MessageTypeAnalyser {
    type Output = Vec<MessageType>;

    fn does_handle(message_type: MessageType) -> bool {
        true
    }

    fn handle_message(&mut self, message: Message, tick: u32) {
        self.packet_types.push(message.get_message_type())
    }

    fn handle_string_entry(&mut self, table: &String, _index: usize, entry: &StringTableEntry) {}

    fn get_output(self, state: ParserState) -> Self::Output {
        self.packet_types
    }
}

impl MessageTypeAnalyser {
    pub fn new() -> Self {
        MessageTypeAnalyser {
            packet_types: Vec::with_capacity(1024)
        }
    }
}