use std::env;
use std::fs;
use bitbuffer::{BigEndian, BitRead, BitReadStream, LittleEndian};
use hound::{SampleFormat, WavSpec, WavWriter};
use main_error::MainError;
use opus::{Channels, Decoder};
use opus::packet::parse;
use steamid_ng::SteamID;
use tf_demo_parser::demo::parser::MessageHandler;
use tf_demo_parser::MessageType;
pub use tf_demo_parser::{Demo, DemoParser, Parse, ParserState};
use tf_demo_parser::demo::data::DemoTick;
use tf_demo_parser::demo::message::Message;
use tf_demo_parser::demo::message::voice::{VoiceInitMessage};

#[cfg(feature = "jemallocator")]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

fn main() -> Result<(), MainError> {
    #[cfg(feature = "better-panic")]
    better_panic::install();

    #[cfg(feature = "trace")]
    tracing_subscriber::fmt::init();

    let args: Vec<_> = env::args().collect();
    if args.len() < 2 {
        println!("1 argument required");
        return Ok(());
    }
    let path = args[1].clone();
    let file = fs::read(path)?;
    let demo = Demo::new(&file);
    let parser = DemoParser::new_with_analyser(demo.get_stream(), Voice::default());
    let (_header, samples) = parser.parse()?;
    let mut sample_buf = vec![0; 16 * 1024];
    let spec = WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create("out.wav", spec).unwrap();
    for sample in samples {
        let packet = decode_steam_voice_sample(&sample.data);
        let mut decoder = Decoder::new(packet.sample_rate as u32, Channels::Mono).unwrap();
        dbg!(&packet.data[0..8]);
        dbg!(parse(&packet.data));
        let samples = decoder.decode(&packet.data, &mut sample_buf, false).unwrap();
        dbg!(samples);
        for sample in &sample_buf[0..samples] {
            writer.write_sample(*sample).unwrap();
        }

    }
    Ok(())
}

#[derive(Default)]
struct Voice {
    pub samples: Vec<VoiceSample>,
    pub last_init: Option<VoiceInitMessage>,
}

#[derive(Debug)]
pub enum VoiceCodec {
    Steam,
    Speex,
    Celt,
    CeltHigh
}

#[derive(Debug)]
struct VoiceSample {
    tick: DemoTick,
    codec: VoiceCodec,
    sampling_rate: u16,
    client: u8,
    data: Vec<u8>,
}

impl MessageHandler for Voice {
    type Output = Vec<VoiceSample>;

    fn does_handle(message_type: MessageType) -> bool {
        matches!(
            message_type,
            MessageType::VoiceInit | MessageType::VoiceData
        )
    }

    fn handle_message(&mut self, message: &Message, tick: DemoTick, _parser_state: &ParserState) {
        match message {
            Message::VoiceInit(init) => {
                self.last_init = Some(init.clone());
            }
            Message::VoiceData(data) => {
                if let Some(init) = self.last_init.as_ref() {
                    let codec = match init.codec.as_str() {
                        "steam" => VoiceCodec::Steam,
                        "vaudio_speex" => VoiceCodec::Speex,
                        "vaudio_celt" => VoiceCodec::Celt,
                        "vaudio_celt_high" => VoiceCodec::CeltHigh,
                        _ => {
                            eprintln!("unknown voice codex: {}", init.codec);
                            return;
                        }
                    };
                    self.samples.push(VoiceSample {
                        tick,
                        codec,
                        sampling_rate: init.sampling_rate,
                        data: data.data.clone().read_sized::<Vec<u8>>(data.length as usize / 8).unwrap(),
                        client: data.client,
                    })
                } else {
                    eprintln!("Voice data without init");
                }
            }
            _ => {}
        }
    }

    fn into_output(self, _state: &ParserState) -> Self::Output {
        self.samples
    }
}

#[derive(BitRead, Debug)]
struct SteamVoiceHeader {
    steam_id: u64,
    ty: u8,
    sample_rate: u16,
    payload_type: u8,
    length: u16,
}

#[derive(Debug)]
struct SteamVoicePacket {
    steam_id: SteamID,
    sample_rate: u16,
    data: Vec<u8>,
}

fn decode_steam_voice_sample(data: &[u8]) -> SteamVoicePacket {
    let mut reader = BitReadStream::<LittleEndian>::from(data);
    let header = reader.read::<SteamVoiceHeader>().unwrap();
    assert_eq!(header.ty, 11);
    let data: Vec<u8> = if header.payload_type == 6 {
        reader.read_sized(header.length as usize).unwrap()
    } else {
        dbg!(header.length, data.len());
        Vec::new()
    };
    SteamVoicePacket {
        steam_id: SteamID::from(header.steam_id),
        sample_rate: header.sample_rate,
        data,
    }
}