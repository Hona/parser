#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]
#![cfg_attr(not(test), deny(clippy::panic))]

extern crate wasm_bindgen;

use std::ffi::{c_char, CStr, CString};
use std::fs;
pub use bitbuffer::Result as ReadResult;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

pub use crate::demo::{
    message::MessageType,
    parser::{
        DemoParser, GameEventError, MatchState, MessageTypeAnalyser, Parse, ParseError,
        ParserState, Result,
    },
    Demo, Stream,
};
use crate::demo::header::Header;

#[cfg(feature = "codegen")]
pub mod codegen;
pub(crate) mod consthash;
pub mod demo;
pub(crate) mod nullhasher;

#[cfg(test)]
#[track_caller]
fn test_roundtrip_write<
    'a,
    T: bitbuffer::BitRead<'a, bitbuffer::LittleEndian>
        + bitbuffer::BitWrite<bitbuffer::LittleEndian>
        + std::fmt::Debug
        + std::cmp::PartialEq,
>(
    val: T,
) {
    let mut data = Vec::with_capacity(128);
    use bitbuffer::{BitReadBuffer, BitReadStream, BitWriteStream, LittleEndian};
    let pos = {
        let mut stream = BitWriteStream::new(&mut data, LittleEndian);
        val.write(&mut stream).unwrap();
        stream.bit_len()
    };

    let mut read = BitReadStream::new(BitReadBuffer::new_owned(data, LittleEndian));
    assert_eq!(
        val,
        read.read().unwrap(),
        "Failed to assert the parsed message is equal to the original"
    );
    assert_eq!(
        pos,
        read.pos(),
        "Failed to assert that all encoded bits ({}) are used for decoding ({})",
        pos,
        read.pos()
    );
}

#[cfg(test)]
#[track_caller]
fn test_roundtrip_encode<
    'a,
    T: Parse<'a> + crate::demo::parser::Encode + std::fmt::Debug + std::cmp::PartialEq,
>(
    val: T,
    state: &ParserState,
) {
    let mut data = Vec::with_capacity(128);
    use bitbuffer::{BitReadBuffer, BitReadStream, BitWriteStream, LittleEndian};
    let pos = {
        let mut stream = BitWriteStream::new(&mut data, LittleEndian);
        val.encode(&mut stream, state).unwrap();
        stream.bit_len()
    };

    let mut read = BitReadStream::new(BitReadBuffer::new_owned(data, LittleEndian));
    pretty_assertions::assert_eq!(
        val,
        T::parse(&mut read, state).unwrap(),
        "Failed to assert the parsed message is equal to the original"
    );
    pretty_assertions::assert_eq!(
        pos,
        read.pos(),
        "Failed to assert that all encoded bits ({}) are used for decoding ({})",
        pos,
        read.pos()
    );
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonDemo {
    header: Header,
    #[serde(flatten)]
    state: MatchState,
}

#[wasm_bindgen(js_namespace = Namespace, catch)]
pub fn analyze_demo(buffer: &[u8]) -> JsValue {
    // Create a Demo object from the buffer
    let demo = Demo::new(buffer);

    // Create a parser from the demo stream
    let parser = DemoParser::new(demo.get_stream());

    // Parse the demo and get the header and state
    let Ok((header, state)) = parser.parse() else {
        return JsValue::from_str("Failed to parse the demo");
    };

    // Create a JsonDemo object
    let demo_json = JsonDemo { header, state };

    // Convert the JsonDemo object to a JSON string
    let Ok(result) = serde_json::to_string(&demo_json) else { return JsValue::from_str("Failed to convert the demo to JSON") };

    // Return the JSON string
    return JsValue::from_str(&result);
}