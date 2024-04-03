//! A machine determines when to inject and/or block outgoing traffic. Consists
//! of one or more [`State`] structs.

use crate::constants::*;
use crate::state::*;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use simple_error::bail;
use std::error::Error;
use std::str::FromStr;
extern crate simple_error;
use base64::prelude::*;
use hex::encode;
use ring::digest::{Context, SHA256};
use std::io::prelude::*;

/// A probabilistic state machine (Rabin automaton) consisting of one or more
/// [`State`] that determine when to inject and/or block outgoing traffic.
#[serde_as]
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub struct Machine {
    /// The number of padding packets the machine is allowed to generate as
    /// actions before other limits apply.
    pub allowed_padding_packets: u64,
    /// The maximum fraction of padding packets to allow as actions.
    pub max_padding_frac: f64,
    /// The number of microseconds of blocking a machine is allowed to generate
    /// as actions before other limits apply.
    pub allowed_blocked_microsec: u64,
    /// The maximum fraction of blocking (microseconds) to allow as actions.
    pub max_blocking_frac: f64,
    /// The states that make up the machine.
    #[serde_as(as = "Vec<State>")]
    pub(crate) states: Vec<StateWrapper>,
}

impl Machine {
    /// Create a new [`Machine`] with the given limits and states. Returns an
    /// error if the machine or any of its states are invalid.
    pub fn new(
        allowed_padding_packets: u64,
        max_padding_frac: f64,
        allowed_blocked_microsec: u64,
        max_blocking_frac: f64,
        states: Vec<State>,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let num_states = states.len();
        let mut wrapped = vec![];

        for s in states.iter() {
            wrapped.push(StateWrapper::new(s.clone(), num_states)?);
        }

        let machine = Machine {
            allowed_padding_packets,
            max_padding_frac,
            allowed_blocked_microsec,
            max_blocking_frac,
            states: wrapped,
        };
        machine.validate()?;

        Ok(machine)
    }

    /// Get a unique and deterministic string that represents the machine. The
    /// string is 32 characters long, hex-encoded.
    pub fn name(&self) -> String {
        let mut context = Context::new(&SHA256);
        context.update(&self.allowed_padding_packets.to_le_bytes());
        context.update(&self.max_padding_frac.to_le_bytes());
        context.update(&self.allowed_blocked_microsec.to_le_bytes());
        context.update(&self.max_blocking_frac.to_le_bytes());

        // We can't just do a json serialization here, because State uses a
        // HashMap, which doesn't guarantee a stable order. Therefore, we add a
        // deterministic print (which is not pretty, but works) for each state,
        // then hash that.
        for state in &self.states {
            context.update(format!("{:?}", state.state).as_bytes());
        }

        let d = context.finish();
        let s = encode(d);
        s[0..32].to_string()
    }

    pub fn serialize(&self) -> String {
        let encoded = bincode::serialize(&self).unwrap();
        let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
        e.write_all(encoded.as_slice()).unwrap();
        let s = BASE64_STANDARD.encode(e.finish().unwrap());
        // version as first 2 characters, then base64 compressed bincoded
        format!("{:02}{}", VERSION, s)
    }

    /// Validates that the machine is in a valid state (machines that are
    /// mutated may get into an invalid state).
    pub fn validate(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        // sane limits
        if self.max_padding_frac < 0.0 || self.max_padding_frac > 1.0 {
            bail!(
                "max_padding_frac has to be [0.0, 1.0], got {}",
                self.max_padding_frac
            )
        }
        if self.max_blocking_frac < 0.0 || self.max_blocking_frac > 1.0 {
            bail!(
                "max_blocking_frac has to be [0.0, 1.0], got {}",
                self.max_blocking_frac
            )
        }

        // sane number of states
        let num_states = self.states.len();

        if num_states == 0 {
            bail!("a machine must have at least one state")
        }
        if num_states > STATE_MAX {
            bail!(
                "too many states, max is {}, found {}",
                STATE_MAX,
                self.states.len()
            )
        }

        // validate all states
        for s in self.states.iter() {
            s.state.validate(num_states)?;
        }

        Ok(())
    }
}

/// from a serialized string, attempt to create a machine
impl FromStr for Machine {
    type Err = Box<dyn Error + Send + Sync>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // version as first 2 characters, then base64
        if s.len() < 3 {
            bail!("string too short")
        }
        let version = &s[0..2];
        if version != format!("{:02}", VERSION) {
            bail!("version mismatch, expected {}, got {}", VERSION, version)
        }
        let s = &s[2..];

        // base64 decoding has a fixed ratio of ~4:3
        let compressed = BASE64_STANDARD.decode(s.as_bytes()).unwrap();
        // decompress, but scared of exceeding memory limits / zlib bombs
        let mut decoder = ZlibDecoder::new(compressed.as_slice());
        let mut buf = vec![0; MAX_DECOMPRESSED_SIZE];
        let bytes_read = decoder.read(&mut buf)?;

        let r = bincode::deserialize(&buf[..bytes_read]);
        Ok(r?)
    }
}

#[cfg(test)]
mod tests {
    use crate::action::*;
    use crate::dist::*;
    use crate::event::Event;
    use crate::machine::*;
    use enum_map::{enum_map, EnumMap};

    #[test]
    fn machine_name_generation() {
        // state 0
        let mut t: EnumMap<Event, Vec<StateTransition>> = enum_map! { _ => vec![] };
        t[Event::PaddingSent].push(StateTransition {
            state: 0,
            probability: 1.0,
        });

        let s0 = State::new(t);

        // machine
        let m = Machine::new(1000, 1.0, 0, 0.0, vec![s0]).unwrap();

        // name generation should be deterministic
        assert_eq!(m.name(), m.name());
    }

    #[test]
    fn validate_machine_limits() {
        // state 0
        let mut t: EnumMap<Event, Vec<StateTransition>> = enum_map! { _ => vec![] };
        t[Event::PaddingSent].push(StateTransition {
            state: 0,
            probability: 1.0,
        });

        let s0 = State::new(t);

        // machine
        let mut m = Machine::new(1000, 1.0, 0, 0.0, vec![s0]).unwrap();

        // max padding frac
        m.max_padding_frac = -0.1;
        let r = m.validate();
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());

        m.max_padding_frac = 1.1;
        let r = m.validate();
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());

        m.max_padding_frac = 0.5;
        let r = m.validate();
        assert!(r.is_ok());

        // max blocking frac
        m.max_blocking_frac = -0.1;
        let r = m.validate();
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());

        m.max_blocking_frac = 1.1;
        let r = m.validate();
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());

        m.max_blocking_frac = 0.5;
        let r = m.validate();
        assert!(r.is_ok());
    }

    #[test]
    fn validate_machine_num_states() {
        // invalid machine lacking state
        let r = Machine::new(1000, 1.0, 0, 0.0, vec![]);

        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());
    }

    #[test]
    fn validate_machine_probability() {
        // out of bounds index
        let mut t: EnumMap<Event, Vec<StateTransition>> = enum_map! { _ => vec![] };
        t[Event::PaddingSent].push(StateTransition {
            state: 1,
            probability: 1.0,
        });

        let s0 = State::new(t);

        // machine with broken state
        let r = Machine::new(1000, 1.0, 0, 0.0, vec![s0.clone()]);
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());

        // try setting one probability too high
        let mut t: EnumMap<Event, Vec<StateTransition>> = enum_map! { _ => vec![] };
        t[Event::PaddingSent].push(StateTransition {
            state: 0,
            probability: 1.1,
        });

        let s0 = State::new(t);

        // machine with broken state
        let r = Machine::new(1000, 1.0, 0, 0.0, vec![s0.clone()]);
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());

        // try setting total probability too high

        // state 0
        let mut t: EnumMap<Event, Vec<StateTransition>> = enum_map! { _ => vec![] };
        t[Event::PaddingSent].push(StateTransition {
            state: 0,
            probability: 0.5,
        });
        t[Event::PaddingSent].push(StateTransition {
            state: 1,
            probability: 0.6,
        });

        let s0 = State::new(t);

        // state 1
        let mut t: EnumMap<Event, Vec<StateTransition>> = enum_map! { _ => vec![] };
        t[Event::PaddingRecv].push(StateTransition {
            state: 1,
            probability: 1.0,
        });

        let s1 = State::new(t);

        // machine with broken state
        let r = Machine::new(1000, 1.0, 0, 0.0, vec![s0, s1]);
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());
    }

    #[test]
    fn validate_machine_distributions() {
        // state 0
        let mut t: EnumMap<Event, Vec<StateTransition>> = enum_map! { _ => vec![] };
        t[Event::PaddingSent].push(StateTransition {
            state: 0,
            probability: 1.0,
        });

        let mut s0 = State::new(t);
        s0.action = Some(Action::SendPadding {
            bypass: false,
            replace: false,
            timeout: Dist {
                dist: DistType::Uniform {
                    low: 10.0,
                    high: 10.0,
                },
                start: 0.0,
                max: 0.0,
            },
            limit: None,
        });

        // valid machine
        let m = Machine::new(1000, 1.0, 0, 0.0, vec![s0.clone()]).unwrap();

        let r = m.validate();
        println!("{:?}", r.as_ref().err());
        assert!(r.is_ok());

        // invalid action in state
        s0.action = Some(Action::SendPadding {
            bypass: false,
            replace: false,
            timeout: Dist {
                dist: DistType::Uniform {
                    low: 2.0, // NOTE param1 > param2
                    high: 1.0,
                },
                start: 0.0,
                max: 0.0,
            },
            limit: None,
        });

        // machine with broken state
        let r = Machine::new(1000, 1.0, 0, 0.0, vec![s0.clone()]);

        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());
    }
}
