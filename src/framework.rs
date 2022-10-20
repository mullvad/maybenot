//! Maybenot is a framework for traffic analysis defenses that can be used to
//! hide patterns in encrypted communication.
//!
//! Consider encrypted communication protocols such as TLS, QUIC, WireGuard, or
//! Tor. While the connections are encrypted, *patterns* in the encrypted
//! communication may still leak information about the underlying plaintext
//! being communicated over encrypted. Maybenot is a framework for creating
//! defenses that hide such patterns.
//!
//!
//! ## Example usage
//! ```
//! use maybenot::{
//! framework::{Action, Framework, TriggerEvent},
//! machine::Machine,
//! };
//! use std::{str::FromStr, time::Instant};
//! // This is a large example usage of the maybenot framework. Some parts are a
//! // bit odd due to avoiding everything async but should convey the general
//! // idea.
//!
//! // Parse machine, this is a "no-op" machine that does nothing. Typically,
//! // you should expect to get one or more serialized machines, not build them
//! // from scratch. The framework takes a vector with zero or more machines as
//! // input when created. To add or remove a machine, just recreate the
//! // framework. If you expect to create many instances of the framework for
//! // the same machines, then share the same vector across framework instances.
//! // All runtime information is allocated internally in the framework without
//! // modifying the machines.
//! let s = "789cedca31010000000141fa9736084080bff9ace928a80003c70003".to_string();
//! // machines will error if invalid
//! let m = Machine::from_str(&s).unwrap();
//!
//! // Create the framework, a lightweight operation, with the following
//! // parameters:
//! // - A vector of zero or more machines.
//! // - Max fractions prevent machines from causing too much overhead: note
//! // that machines can be defined to be allowed a fixed amount of
//! // padding/blocking, bypassing these limits until having used up their
//! // allowed budgets. This means that it is possible to create machines that
//! // trigger actions to block outgoing traffic indefinitely and/or send a lot
//! // of outgoing traffic.
//! // - The current MTU of the link being protected. It can be updated later by
//! // triggering TriggerEvent::UpdateMTU { new_mtu: u16 }.
//! // - The current time. For normal use, just provide the current time as
//! // below. This is exposed mainly for testing purposes (can also be used to
//! // make the creation of some odd types of machines easier).
//! //
//! // The framework validates all machines (like fn parse_machine() above) so
//! // it can error out.
//! let mut f = Framework::new(vec![m], 0.0, 0.0, 1420, Instant::now()).unwrap();
//!
//! // Below is the main loop for operating the framework. This should run for
//! // as long as the underlying connection the framework is attached to can
//! // communicate (user or protocol-specific data, depending on what is being
//! // defended).
//! loop {
//!     // Wait for one or more new events (e.g., on a channel) that should be
//!     // triggered in the framework. Below we just set one example event. How
//!     // you wait and collect events is likely going to be a bottleneck. If
//!     // you have to consider dropping events, it is better to drop older
//!     // events than newer.
//!     let events = [TriggerEvent::NonPaddingSent { bytes_sent: 1420 }];
//!
//!     // Trigger the events in the framework. This takes linear time with the
//!     // number of events but is very fast (time should be dominated by at
//!     // most four calls to sample randomness per event per machine).
//!     for action in f.trigger_events(&events, Instant::now()) {
//!         // After triggering all the events, the framework will provide zero
//!         // or more actions to take, up to a maximum of one action per
//!         // machine (regardless of the number of events). It is your
//!         // responsibility to perform those actions according to the
//!         // specification. To do so, you will need up to one timer per
//!         // machine. The machine identifier (machine in each Action) uniquely
//!         // and deterministically maps to a single machine running in the
//!         // framework (so suitable as a key for a data structure storing your
//!         // timers, e.g., a HashMap<MachineId, SomeTimerDataStructure>).
//!         match action {
//!             Action::Cancel { machine: _ } => {
//!                 // If any active pending timer for this machine, cancel it.
//!             }
//!             Action::InjectPadding {
//!                 timeout: _,
//!                 size: _,
//!                 machine: _,
//!             } => {
//!                 // Set the timer with the specified timeout. On expiry, do
//!                 // the following (all of nothing):
//!                 // 1. Send size padding.
//!                 // 2. Add TriggerEvent::PaddingSent{ bytes_sent: size,
//!                 //    machine: machine } to be triggered next loop
//!                 //    iteration.
//!                 //
//!                 // Above, "send" should mimic as close as possible real
//!                 // application data being added for transport. Also, note
//!                 // that if there already is a timer for an earlier action
//!                 // for the machine index in question, replace it. This will
//!                 // happen very frequently so make effort to make it
//!                 // efficient (typically, efficient machines will always have
//!                 // something scheduled but try to minimize actual padding
//!                 // sent, i.e., expired timers).
//!             }
//!             Action::BlockOutgoing {
//!                 timeout: _,
//!                 duration: _,
//!                 overwrite: _,
//!                 machine: _,
//!             } => {
//!                 // Set the timer with the specified timeout, overwriting any
//!                 // existing action timer for the machine (be it to block or
//!                 // inject). On expiry, do the following (all or nothing):
//!                 // 1. If no blocking is currently taking place (globally
//!                 //    across all machines, so for this instance of the
//!                 //    framework), start blocking all outgoing traffic for
//!                 //    the specified duration. If blocking is already taking
//!                 //    place (due to any machine), there are two cases. If
//!                 //    overwrite is true, replace the existing blocking
//!                 //    duration with the specified duration in this action.
//!                 //    If overwrite is false, pick the longest duration of
//!                 //    the specified duration and the *remaining* duration to
//!                 //    block already in place.
//!                 // 2. Add TriggerEvent::BlockingBegin { machine: machine }
//!                 //    to be triggered next loop iteration (regardless of
//!                 //    logic outcome in 1, from the point of view of the
//!                 //    machine, blocking is now taking place).
//!                 //
//!                 // Note that blocking is global across all machines, since
//!                 // the intent is to block all outgoing traffic. Further, you
//!                 // MUST ensure that when blocking ends, you add
//!                 // TriggerEvent::BlockingEnd to be triggered next loop
//!                 // iteration.
//!             }
//!         }
//!     }
//!
//!     // All done, continue the loop. We break below for the example test to
//!     // not get stuck.
//!     break;
//! }
//! ```
use simple_error::bail;

use crate::constants::*;
use crate::dist::DistType;
use crate::event::*;
use crate::machine::*;
use std::error::Error;
use std::fmt;
use std::time::Duration;
use std::time::Instant;

/// An opaque token representing one machine running inside the framework.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct MachineId(usize);

/// Represents an event to be triggered in the framework.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum TriggerEvent {
    /// Received non-padding bytes.
    NonPaddingRecv { bytes_recv: u16 },
    /// Received padding bytes.
    PaddingRecv { bytes_recv: u16 },
    /// Sent non-padding bytes.
    NonPaddingSent { bytes_sent: u16 },
    /// Sent padding bytes.
    PaddingSent { bytes_sent: u16, machine: MachineId },
    /// Blocking of outgoing traffic started by the action from a machine.
    BlockingBegin { machine: MachineId },
    /// Blocking of outgoing traffic stopped.
    BlockingEnd,
    /// An event triggered internally by the framework when a state has used up
    /// its limit.
    LimitReached { machine: MachineId },
    /// The MTU of the protected connection was updated.
    UpdateMTU { new_mtu: u16 },
}

impl TriggerEvent {
    /// Checks if the [`TriggerEvent`] is a particular [`Event`].
    pub fn is_event(&self, e: Event) -> bool {
        match self {
            TriggerEvent::NonPaddingRecv { .. } => e == Event::NonPaddingRecv,
            TriggerEvent::PaddingRecv { .. } => e == Event::PaddingRecv,
            TriggerEvent::NonPaddingSent { .. } => e == Event::NonPaddingSent,
            TriggerEvent::PaddingSent { .. } => e == Event::PaddingSent,
            TriggerEvent::BlockingBegin { .. } => e == Event::BlockingBegin,
            TriggerEvent::BlockingEnd => e == Event::BlockingEnd,
            TriggerEvent::LimitReached { .. } => e == Event::LimitReached,
            TriggerEvent::UpdateMTU { .. } => e == Event::UpdateMTU,
        }
    }
}

impl fmt::Display for TriggerEvent {
    // note that we don't share the private MachineId
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TriggerEvent::NonPaddingRecv { bytes_recv } => write!(f, "rn,{}", bytes_recv),
            TriggerEvent::PaddingRecv { bytes_recv } => write!(f, "rp,{}", bytes_recv),
            TriggerEvent::NonPaddingSent { bytes_sent } => write!(f, "sn,{}", bytes_sent),
            TriggerEvent::PaddingSent { bytes_sent, .. } => write!(f, "sp,{}", bytes_sent),
            TriggerEvent::BlockingBegin { .. } => write!(f, "bb"),
            TriggerEvent::BlockingEnd => write!(f, "be"),
            TriggerEvent::LimitReached { .. } => write!(f, "lr"),
            TriggerEvent::UpdateMTU { new_mtu } => write!(f, "um,{}", new_mtu),
        }
    }
}

/// The action to be taken by the framework user.
#[derive(PartialEq, Debug, Clone)]
pub enum Action {
    /// Stop any currently scheduled action for the machine.
    Cancel { machine: MachineId },
    /// Schedule padding to be injected after the given timeout for the machine.
    /// The size of the padding (in bytes) is specified. Will never be larger
    /// than MTU.
    InjectPadding {
        timeout: Duration,
        size: u16,
        machine: MachineId,
    },
    /// Schedule outgoing traffic to be blocked after the given timeout for the
    /// machine. The duration of the block is specified as well as a flag
    /// indicating of the duration should overwrite any other currently ongoing
    /// blocking of outgoing traffic.
    BlockOutgoing {
        timeout: Duration,
        duration: Duration,
        overwrite: bool,
        machine: MachineId,
    },
}

#[derive(Debug, Clone)]
struct MachineRuntime {
    current_state: usize,
    state_limit: u64,
    padding_sent: u64,
    nonpadding_sent: u64,
    blocking_duration: Duration,
    machine_start: Instant,
}

#[derive(PartialEq)]
enum StateChange {
    Changed,
    Unchanged,
}
/// An instance of the Maybenot framework.
///
/// An instance of the [`Framework`] repeatedly takes as *input* one or more
/// [`TriggerEvent`] describing the encrypted traffic going over an encrypted
/// channel, and produces as *output* zero or more [`Action`], such as to inject
/// *padding* traffic or *block* outgoing traffic. One or more [`Machine`]
/// determine what [`Action`] to take based on [`TriggerEvent`].
pub struct Framework {
    actions: Vec<Option<Action>>,
    current_time: Instant,
    machines: Vec<Machine>,
    runtime: Vec<MachineRuntime>,
    global_max_padding_frac: f64,
    global_nonpadding_sent_bytes: u64,
    global_paddingsent_bytes: u64,
    global_max_blocking_frac: f64,
    global_blocking_duration: Duration,
    global_blocking_started: Instant,
    global_blocking_active: bool,
    global_framework_start: Instant,
    mtu: u16,
}

impl Framework {
    /// Create a new framework instance with zero or more [`Machine`]. The max
    /// padding/blocking fractions are enforced as a total across all machines.
    /// The only way those limits can be violated are through
    /// [`Machine::allowed_padding_bytes`] and
    /// [`Machine::allowed_blocked_microsec`], respectively. The MTU is the MTU
    /// of the underlying connection (goodput). The current time is handed to
    /// the framework here (and later in [`Self::trigger_events()`]) to make
    /// some types of use-cases of the framework easier (weird machines and for
    /// simulation). Returns an error on any invalid [`Machine`] or limits not
    /// being fractions [0, 1.0].
    pub fn new(
        machines: Vec<Machine>,
        max_padding_frac: f64,
        max_blocking_frac: f64,
        mtu: u16,
        current_time: Instant,
    ) -> Result<Self, Box<dyn Error>> {
        for m in &machines {
            m.validate()?;
        }

        if max_padding_frac < 0.0 || max_padding_frac > 1.0 {
            bail!("max_padding_frac has to be beteen [0.0, 1.0]");
        }
        if max_blocking_frac < 0.0 || max_blocking_frac > 1.0 {
            bail!("max_blocking_frac has to be beteen [0.0, 1.0]");
        }

        let runtime = vec![
            MachineRuntime {
                current_state: 0,
                state_limit: 0,
                padding_sent: 0,
                nonpadding_sent: 0,
                blocking_duration: Duration::from_secs(0),
                machine_start: current_time.clone(),
            };
            machines.len()
        ];

        let actions = vec![None; machines.len()];

        Ok(Self {
            actions,
            machines,
            runtime,
            mtu,
            current_time,
            global_max_blocking_frac: max_blocking_frac,
            global_max_padding_frac: max_padding_frac,
            global_framework_start: current_time.clone(),
            global_blocking_active: false,
            global_blocking_started: current_time.clone(), // ugly, can't be unset
            global_blocking_duration: Duration::from_secs(0),
            global_paddingsent_bytes: 0,
            global_nonpadding_sent_bytes: 0,
        })
    }

    /// Trigger zero or more [`TriggerEvent`] for all machines running in the
    /// framework. The current time SHOULD be the current time at time of
    /// calling the method (e.g., [`Instant::now()`]). Returns an iterator of
    /// zero or more [`Action`] that MUST be taken by the caller.
    pub fn trigger_events(
        &mut self,
        events: &[TriggerEvent],
        current_time: Instant,
    ) -> impl Iterator<Item = &Action> {
        // reset all actions
        self.actions.fill(None);

        // Process all events: note that each event may lead to up to one action
        // per machine, but that future events may overwrite those actions.
        // Under load, this is preferable (because something already happened
        // before we could cause an action, so better to catch up).
        self.current_time = current_time;
        for e in events {
            self.process_event(&e);
        }

        // only return actions, no None
        self.actions.iter().filter_map(|action| action.as_ref())
    }

    fn process_event(&mut self, e: &TriggerEvent) {
        match e {
            TriggerEvent::NonPaddingRecv { bytes_recv } => {
                // no special accounting needed
                for mi in 0..self.runtime.len() {
                    self.transition(mi, Event::NonPaddingRecv, *bytes_recv as u64);
                }
            }
            TriggerEvent::PaddingRecv { bytes_recv } => {
                // no special accounting needed
                for mi in 0..self.runtime.len() {
                    self.transition(mi, Event::PaddingRecv, *bytes_recv as u64);
                }
            }
            TriggerEvent::NonPaddingSent { bytes_sent } => {
                self.global_nonpadding_sent_bytes += *bytes_sent as u64;

                for mi in 0..self.runtime.len() {
                    self.runtime[mi].nonpadding_sent += *bytes_sent as u64;

                    // If the transition leaves the state unchanged and the limit of
                    // the machine includes nonpadding sent packets, decrement the
                    // limit. If the state changed, a new limit was sampled and this
                    // packet shouldn't count.
                    if self.transition(mi, Event::NonPaddingSent, *bytes_sent as u64)
                        == StateChange::Unchanged
                    {
                        let cs = self.runtime[mi].current_state;
                        if cs != STATEEND {
                            if self.machines[mi].states[cs].limit_includes_nonpadding {
                                self.decrement_limit(mi);
                            }
                        }
                    }
                }
            }
            TriggerEvent::PaddingSent {
                bytes_sent,
                machine,
            } => {
                // accounting is global ...
                self.global_paddingsent_bytes += *bytes_sent as u64;

                for mi in 0..self.runtime.len() {
                    // ... but the event is per-machine
                    // TODO: we probably want a PaddingQueued (self) and PaddingSent (global)
                    if mi == machine.0 {
                        self.runtime[mi].padding_sent += *bytes_sent as u64;

                        if self.transition(mi, Event::PaddingSent, *bytes_sent as u64)
                            == StateChange::Unchanged
                        {
                            // decrement only makes sense if we didn't change state
                            self.decrement_limit(mi)
                        }
                    }
                }
            }
            TriggerEvent::BlockingBegin { machine } => {
                // keep track of when we start blocking (for accounting in BlockingEnd)
                if !self.global_blocking_active {
                    self.global_blocking_active = true;
                    self.global_blocking_started = self.current_time.clone();
                }

                // blocking is a global event
                for mi in 0..self.runtime.len() {
                    if self.transition(mi, Event::BlockingBegin, 0) == StateChange::Unchanged {
                        if mi == machine.0 {
                            // decrement only makes sense if we didn't
                            // change state and for the machine in question
                            self.decrement_limit(mi)
                        }
                    }
                }
            }
            TriggerEvent::BlockingEnd => {
                let mut blocked: Duration = Duration::from_secs(0);
                if self.global_blocking_active {
                    blocked = self
                        .current_time
                        .duration_since(self.global_blocking_started);
                    self.global_blocking_duration += blocked;
                    self.global_blocking_active = false;
                }

                for mi in 0..self.runtime.len() {
                    // since block is global, every machine was blocked the
                    // same duration
                    if !blocked.is_zero() {
                        self.runtime[mi].blocking_duration += blocked;
                    }
                    self.transition(mi, Event::BlockingEnd, 0);
                }
            }
            TriggerEvent::LimitReached { machine } => {
                // limit is an internal event
                self.transition(machine.0, Event::LimitReached, 0);
            }
            TriggerEvent::UpdateMTU { new_mtu } => {
                self.mtu = *new_mtu;
                for mi in 0..self.runtime.len() {
                    self.transition(mi, Event::UpdateMTU, *new_mtu as u64);
                }
            }
        };
    }

    // FIXME: should probably just return the action instead and set mi outside?
    fn transition(&mut self, mi: usize, event: Event, n: u64) -> StateChange {
        let machine = &self.machines[mi];

        // a machine in end state cannot transition
        if self.runtime[mi].current_state == STATEEND {
            return StateChange::Unchanged;
        }

        // ignore events generated by small packets if not included
        if !machine.include_small_packets && n > 0 && n <= MAXSMALLPACKETSIZE {
            return StateChange::Unchanged;
        }

        // sample next state
        let (next_state, set) = self.next_state(&self.runtime[mi], machine, event);

        // if no next state on event, done
        if !set {
            return StateChange::Unchanged;
        }

        // we got a next state, act on it
        match next_state {
            STATECANCEL => {
                // cancel any pending action, but doesn't count as a state change
                self.actions[mi] = Some(Action::Cancel {
                    machine: MachineId(mi),
                });
                return StateChange::Unchanged;
            }
            STATEEND => {
                // this is a state change (because we can never reach here if already in
                // STATEEND, see first check above), but we don't cancel any pending
                // action, nor schedule any new action
                self.runtime[mi].current_state = STATEEND;
                return StateChange::Changed;
            }
            _ => {
                // transition to same or different state?
                if self.runtime[mi].current_state == next_state {
                    if self.below_action_limits(&self.runtime[mi], machine) {
                        self.actions[mi] =
                            self.schedule_action(&self.runtime[mi], machine, MachineId(mi));
                    }
                    return StateChange::Unchanged;
                }
                self.runtime[mi].current_state = next_state;
                self.runtime[mi].state_limit = machine.states[next_state].sample_limit();
                if self.below_action_limits(&self.runtime[mi], machine) {
                    self.actions[mi] =
                        self.schedule_action(&self.runtime[mi], machine, MachineId(mi));
                }
                return StateChange::Changed;
            }
        }
    }

    fn schedule_action(
        &self,
        runtime: &MachineRuntime,
        machine: &Machine,
        mi: MachineId,
    ) -> Option<Action> {
        let current = &machine.states[runtime.current_state];

        if current.action_is_block {
            Some(Action::BlockOutgoing {
                timeout: Duration::from_micros(current.sample_timeout() as u64),
                duration: Duration::from_micros(current.sample_block() as u64),
                overwrite: current.block_overwrite,
                machine: mi,
            })
        } else {
            Some(Action::InjectPadding {
                timeout: Duration::from_micros(current.sample_timeout() as u64),
                size: current.sample_size(self.mtu as u64) as u16,
                machine: mi,
            })
        }
    }

    fn decrement_limit(&mut self, mi: usize) {
        if self.runtime[mi].state_limit > 0 {
            self.runtime[mi].state_limit -= 1;
        }
        let cs = self.runtime[mi].current_state;

        if self.runtime[mi].state_limit == 0
            && self.machines[mi].states[cs].limit.dist != DistType::None
        {
            // take no action and trigger limit reached
            self.actions[mi] = None;
            // next, we trigger internally event LimitReached
            self.process_event(&TriggerEvent::LimitReached {
                machine: MachineId(mi),
            })
        }
    }

    fn next_state(
        &self,
        runtime: &MachineRuntime,
        machine: &Machine,
        event: Event,
    ) -> (usize, bool) {
        if !machine.states[runtime.current_state]
            .next_state
            .contains_key(&event)
        {
            return (0, false);
        }
        let next_prop = &machine.states[runtime.current_state].next_state[&event];

        let p = rand::random::<f64>();
        let mut total = 0.0;
        for i in 0..next_prop.len() {
            total += next_prop[i];
            if p <= total {
                // some events are machine-defined, others framework pseudo-states
                if i + 2 < next_prop.len() {
                    return (i, true);
                } else if i + 2 == next_prop.len() {
                    return (STATECANCEL, true);
                } else {
                    return (STATEEND, true);
                }
            }
        }

        (STATENOP, false)
    }

    fn below_action_limits(&self, runtime: &MachineRuntime, machine: &Machine) -> bool {
        let current = &machine.states[runtime.current_state];
        // either blocking or padding limits apply
        if current.action_is_block {
            return self.below_limit_blocking(runtime, machine);
        }
        self.below_limit_padding(runtime, machine)
    }

    fn below_limit_blocking(&self, runtime: &MachineRuntime, machine: &Machine) -> bool {
        let current = &machine.states[runtime.current_state];
        // blocking action

        // special case: we always allow overwriting existing blocking
        if current.block_overwrite && !self.global_blocking_active {
            // we still check against sate limit, because its machine internal
            return runtime.state_limit > 0;
        }

        // compute durations we've been blocking
        let mut m_block_dur = runtime.blocking_duration;
        let mut g_block_dur = self.global_blocking_duration;
        if self.global_blocking_active {
            // account for ongoing blocking as well, add duration
            m_block_dur += self
                .current_time
                .duration_since(self.global_blocking_started);
            g_block_dur += self
                .current_time
                .duration_since(self.global_blocking_started);
        }

        // machine allowed blocking duration first, since it bypasses the
        // other two types of limits
        if m_block_dur < Duration::from_micros(machine.allowed_blocked_microsec) {
            // we still check against sate limit, because its machine internal
            return runtime.state_limit > 0;
        }

        // does the machine limit say no, if set?
        if machine.max_blocking_frac > 0.0 {
            // TODO: swap to m_block_dur.div_duration_f64()
            let f: f64 = m_block_dur.as_micros() as f64
                / self
                    .current_time
                    .duration_since(runtime.machine_start)
                    .as_micros() as f64;
            if f >= machine.max_blocking_frac {
                return false;
            }
        }

        // does the framework say no?
        if self.global_max_blocking_frac > 0.0 {
            // TODO: swap to g_block_dur.div_duration_f64()
            let f: f64 = g_block_dur.as_micros() as f64
                / self
                    .current_time
                    .duration_since(self.global_framework_start)
                    .as_micros() as f64;
            if f >= self.global_max_blocking_frac {
                return false;
            }
        }

        // only state-limit left to consider
        return runtime.state_limit > 0;
    }

    fn below_limit_padding(&self, runtime: &MachineRuntime, machine: &Machine) -> bool {
        // no limits apply if not made up padding count
        if runtime.padding_sent < machine.allowed_padding_bytes {
            return runtime.state_limit > 0;
        }

        // hit machine limits?
        if machine.max_padding_frac > 0.0 {
            let total = runtime.nonpadding_sent + runtime.padding_sent;
            if total == 0 {
                // FIXME: edge-case, was defined as false in go-framework, but should be true?
                return false;
            }
            if runtime.padding_sent as f64 / total as f64 >= machine.max_padding_frac {
                return false;
            }
        }

        // hit global limits?
        if self.global_max_padding_frac > 0.0 {
            let frac = self.global_paddingsent_bytes as f64
                / (self.global_paddingsent_bytes as f64 + self.global_nonpadding_sent_bytes as f64);
            if frac >= self.global_max_padding_frac {
                return false;
            }
        }

        // only state-limit left to consider
        return runtime.state_limit > 0;
    }
}

#[cfg(test)]
mod tests {
    use crate::dist::*;
    use crate::framework::*;
    use crate::state::*;
    use std::collections::HashMap;
    use std::ops::Add;
    use std::time::Duration;
    use std::time::Instant;

    #[test]
    fn no_machines() {
        let f = Framework::new(vec![], 0.0, 0.0, 150, Instant::now());
        assert!(!f.is_err());
    }

    #[test]
    fn trigger_events_actions() {
        // plan: create a machine that swaps between two states, trigger one
        // then multiple events and check the resulting actions

        let num_states = 2;

        // state 0: go to state 1 on PaddingSent, pad after 10 usec
        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(1, 1.0);
        t.insert(Event::PaddingSent, e);
        let mut s0 = State::new(t, num_states);
        s0.timeout = Dist {
            dist: DistType::Uniform,
            param1: 10.0,
            param2: 10.0,
            start: 0.0,
            max: 0.0,
        };

        // state 1: go to state 0 on PaddingRecv, pad after 1 usec
        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(0, 1.0);
        t.insert(Event::PaddingRecv, e);
        let mut s1 = State::new(t, num_states);
        s1.timeout = Dist {
            dist: DistType::Uniform,
            param1: 1.0,
            param2: 1.0,
            start: 0.0,
            max: 0.0,
        };

        // create a simple machine
        let m = Machine {
            allowed_padding_bytes: 1000 * 1024,
            max_padding_frac: 1.0,
            allowed_blocked_microsec: 0,
            max_blocking_frac: 0.0,
            states: vec![s0, s1],
            include_small_packets: true,
        };

        let mut current_time = Instant::now();
        let mtu = 150;
        let mut f = Framework::new(vec![m], 0.0, 0.0, mtu, current_time).unwrap();

        assert_eq!(f.actions.len(), 1);

        // start triggering
        _ = f.trigger_events(
            &[TriggerEvent::BlockingBegin {
                machine: MachineId(0),
            }],
            current_time,
        );
        assert_eq!(f.actions[0], None);

        // move time forward, trigger again to make sure no scheduled timer
        current_time = current_time.add(Duration::from_micros(20));
        _ = f.trigger_events(
            &[TriggerEvent::BlockingBegin {
                machine: MachineId(0),
            }],
            current_time,
        );
        assert_eq!(f.actions[0], None);

        // trigger transition to next state
        _ = f.trigger_events(
            &[TriggerEvent::PaddingSent {
                bytes_sent: 0,
                machine: MachineId(0),
            }],
            current_time,
        );
        assert_eq!(
            f.actions[0],
            Some(Action::InjectPadding {
                timeout: Duration::from_micros(1),
                size: mtu as u16,
                machine: MachineId(0),
            })
        );

        // increase time, trigger event, make sure no further action
        current_time = current_time.add(Duration::from_micros(20));
        _ = f.trigger_events(
            &[TriggerEvent::PaddingSent {
                bytes_sent: 0,
                machine: MachineId(0),
            }],
            current_time,
        );
        assert_eq!(f.actions[0], None);

        // go back to state 0
        _ = f.trigger_events(&[TriggerEvent::PaddingRecv { bytes_recv: 0 }], current_time);
        assert_eq!(
            f.actions[0],
            Some(Action::InjectPadding {
                timeout: Duration::from_micros(10),
                size: mtu as u16,
                machine: MachineId(0),
            })
        );

        // test multiple triggers overwriting actions
        for _ in 0..10 {
            _ = f.trigger_events(
                &[
                    TriggerEvent::PaddingSent {
                        bytes_sent: 0,
                        machine: MachineId(0),
                    },
                    TriggerEvent::PaddingRecv { bytes_recv: 0 },
                ],
                current_time,
            );
            assert_eq!(
                f.actions[0],
                Some(Action::InjectPadding {
                    timeout: Duration::from_micros(10),
                    size: mtu as u16,
                    machine: MachineId(0),
                })
            );
        }

        // triple trigger, swapping between states
        for i in 0..10 {
            if i % 2 == 0 {
                _ = f.trigger_events(
                    &[
                        TriggerEvent::PaddingRecv { bytes_recv: 0 },
                        TriggerEvent::PaddingSent {
                            bytes_sent: 0,
                            machine: MachineId(0),
                        },
                        TriggerEvent::PaddingRecv { bytes_recv: 0 },
                    ],
                    current_time,
                );
                assert_eq!(
                    f.actions[0],
                    Some(Action::InjectPadding {
                        timeout: Duration::from_micros(10),
                        size: mtu as u16,
                        machine: MachineId(0),
                    })
                );
            } else {
                _ = f.trigger_events(
                    &[
                        TriggerEvent::PaddingSent {
                            bytes_sent: 0,
                            machine: MachineId(0),
                        },
                        TriggerEvent::PaddingRecv { bytes_recv: 0 },
                        TriggerEvent::PaddingSent {
                            bytes_sent: 0,
                            machine: MachineId(0),
                        },
                    ],
                    current_time,
                );
                assert_eq!(
                    f.actions[0],
                    Some(Action::InjectPadding {
                        timeout: Duration::from_micros(1),
                        size: mtu as u16,
                        machine: MachineId(0),
                    })
                );
            }
        }
    }

    #[test]
    fn validate_machine() {
        let num_states = 1;

        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        // invalid state transition
        e.insert(1, 1.0);
        t.insert(Event::PaddingSent, e);
        let mut s0 = State::new(t, num_states);
        s0.timeout = Dist {
            dist: DistType::Uniform,
            param1: 10.0,
            param2: 10.0,
            start: 0.0,
            max: 0.0,
        };

        // machine with broken state
        let m = Machine {
            allowed_padding_bytes: 1000 * 1024,
            max_padding_frac: 1.0,
            allowed_blocked_microsec: 0,
            max_blocking_frac: 0.0,
            states: vec![s0.clone()],
            include_small_packets: true,
        };
        // while we get an error here, as intended, the error is not the
        // expected one, because make_next_state() actually ignores the
        // transition to the non-existing state as it makes the probability
        // matrix based on num_states
        let r = m.validate();
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());

        // repair state
        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(0, 1.0);
        t.insert(Event::PaddingSent, e);
        s0.next_state = make_next_state(t, num_states);
        let m = Machine {
            allowed_padding_bytes: 1000 * 1024,
            max_padding_frac: 1.0,
            allowed_blocked_microsec: 0,
            max_blocking_frac: 0.0,
            states: vec![s0.clone()],
            include_small_packets: true,
        };

        let r = m.validate();
        println!("{:?}", r.as_ref().err());
        assert!(r.is_ok());

        // invalid machine lacking state
        let m = Machine {
            allowed_padding_bytes: 1000 * 1024,
            max_padding_frac: 1.0,
            allowed_blocked_microsec: 0,
            max_blocking_frac: 0.0,
            states: vec![],
            include_small_packets: true,
        };
        let r = m.validate();
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());

        // bad padding and blocking fractions
        let mut m = Machine {
            allowed_padding_bytes: 1000 * 1024,
            max_padding_frac: 1.0,
            allowed_blocked_microsec: 0,
            max_blocking_frac: 0.0,
            states: vec![s0.clone()],
            include_small_packets: true,
        };

        m.max_padding_frac = -0.1;
        let r = m.validate();
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());
        m.max_padding_frac = 1.1;
        let r = m.validate();
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());
        m.max_padding_frac = 0.5;

        m.max_blocking_frac = -0.1;
        let r = m.validate();
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());
        m.max_blocking_frac = 1.1;
        let r = m.validate();
        println!("{:?}", r.as_ref().err());
        assert!(r.is_err());
        m.max_blocking_frac = 0.5;
    }

    #[test]
    fn blocking_machine() {
        // a machine that blocks for 10us, 1us after EventNonPaddingSent
        let num_states = 2;

        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(1, 1.0);
        t.insert(Event::NonPaddingSent, e);
        let mut s0 = State::new(t, num_states);
        s0.timeout = Dist {
            dist: DistType::Uniform,
            param1: 0.0,
            param2: 0.0,
            start: 0.0,
            max: 0.0,
        };
        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(1, 1.0);
        t.insert(Event::NonPaddingSent, e);
        let mut s1 = State::new(t, num_states);
        s1.timeout = Dist {
            dist: DistType::Uniform,
            param1: 1.0,
            param2: 1.0,
            start: 0.0,
            max: 0.0,
        };
        s1.action = Dist {
            dist: DistType::Uniform,
            param1: 10.0,
            param2: 10.0,
            start: 0.0,
            max: 0.0,
        };
        s1.action_is_block = true;

        let m = Machine {
            allowed_padding_bytes: 1000 * 1024,
            max_padding_frac: 1.0,
            allowed_blocked_microsec: 0,
            max_blocking_frac: 0.0,
            states: vec![s0, s1],
            include_small_packets: true,
        };

        let mut current_time = Instant::now();
        let mtu = 150;
        let mut f = Framework::new(vec![m], 0.0, 0.0, mtu, current_time).unwrap();

        _ = f.trigger_events(
            &[TriggerEvent::NonPaddingSent { bytes_sent: 0 }],
            current_time,
        );
        assert_eq!(
            f.actions[0],
            Some(Action::BlockOutgoing {
                timeout: Duration::from_micros(1),
                duration: Duration::from_micros(10),
                overwrite: false,
                machine: MachineId(0),
            })
        );

        current_time = current_time.add(Duration::from_micros(20));
        _ = f.trigger_events(
            &[TriggerEvent::BlockingBegin {
                machine: MachineId(0),
            }],
            current_time,
        );
        assert_eq!(f.actions[0], None);

        for _ in 0..10 {
            current_time = current_time.add(Duration::from_micros(1));
            _ = f.trigger_events(
                &[TriggerEvent::NonPaddingSent { bytes_sent: 0 }],
                current_time,
            );
            assert_eq!(
                f.actions[0],
                Some(Action::BlockOutgoing {
                    timeout: Duration::from_micros(1),
                    duration: Duration::from_micros(10),
                    overwrite: false,
                    machine: MachineId(0),
                })
            );
        }
    }

    #[test]
    fn machine_max_padding_frac() {
        // We create a machine that should be allowed to send 100*MTU padding
        // bytes before machine padding limits are applied, then the machine
        // should be limited from sending any padding until at least 100*MTU
        // nonpadding bytes have been sent, given the set max padding fraction
        // of 0.5.
        let mtu: u16 = 1000;

        let num_states = 2;
        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(1, 1.0);
        t.insert(Event::NonPaddingRecv, e);
        let s0 = State::new(t, num_states);

        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(1, 1.0);
        // we use sent for checking limits
        t.insert(Event::PaddingSent, e.clone());
        t.insert(Event::NonPaddingSent, e.clone());
        // recv as an event to check without adding bytes sent
        t.insert(Event::NonPaddingRecv, e.clone());
        let mut s1 = State::new(t, num_states);
        s1.timeout = Dist {
            dist: DistType::Uniform,
            param1: 2.0,
            param2: 2.0,
            start: 0.0,
            max: 0.0,
        };

        let m = Machine {
            allowed_padding_bytes: 100 * (mtu as u64),
            max_padding_frac: 0.5,
            allowed_blocked_microsec: 0,
            max_blocking_frac: 0.0,
            states: vec![s0, s1],
            include_small_packets: true,
        };
        let current_time = Instant::now();
        let mut f = Framework::new(vec![m], 0.0, 0.0, mtu, current_time).unwrap();

        // transition to get the loop going
        _ = f.trigger_events(
            &[TriggerEvent::NonPaddingRecv { bytes_recv: 0 }],
            current_time,
        );

        // we expect 100 padding actions
        for _ in 0..100 {
            assert_eq!(
                f.actions[0],
                Some(Action::InjectPadding {
                    timeout: Duration::from_micros(2),
                    size: mtu as u16,
                    machine: MachineId(0),
                })
            );

            _ = f.trigger_events(
                &[TriggerEvent::PaddingSent {
                    bytes_sent: mtu as u16,
                    machine: MachineId(0),
                }],
                current_time,
            );
        }

        // limit hit, last event should prevent the action
        assert_eq!(f.actions[0], None);

        // trigger and check limit again
        _ = f.trigger_events(
            &[TriggerEvent::NonPaddingRecv {
                bytes_recv: mtu as u16,
            }],
            current_time,
        );
        assert_eq!(f.actions[0], None);

        // verify that no padding is scheduled until we've sent the same amount
        // of bytes
        for _ in 0..100 {
            _ = f.trigger_events(
                &[TriggerEvent::NonPaddingSent {
                    bytes_sent: mtu as u16,
                }],
                current_time,
            );
            assert_eq!(f.actions[0], None);
        }

        // send one byte of nonpadding, putting us just over the limit
        _ = f.trigger_events(
            &[TriggerEvent::NonPaddingSent { bytes_sent: 1 }],
            current_time,
        );
        assert_eq!(
            f.actions[0],
            Some(Action::InjectPadding {
                timeout: Duration::from_micros(2),
                size: mtu as u16,
                machine: MachineId(0),
            })
        );
    }

    #[test]
    fn framework_max_padding_frac() {
        // to test the global limits of the framework we create two machines with
        // the same allowed padding, where both machines pad in parallel
        let mtu: u16 = 1000;

        let num_states = 2;
        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(1, 1.0);
        t.insert(Event::NonPaddingRecv, e);
        let s0 = State::new(t, num_states);
        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(1, 1.0);
        // we use sent for checking limits
        t.insert(Event::PaddingSent, e.clone());
        t.insert(Event::NonPaddingSent, e.clone());
        // recv as an event to check without adding bytes sent
        t.insert(Event::NonPaddingRecv, e.clone());
        let mut s1 = State::new(t, num_states);
        s1.timeout = Dist {
            dist: DistType::Uniform,
            param1: 2.0,
            param2: 2.0,
            start: 0.0,
            max: 0.0,
        };

        let m1 = Machine {
            allowed_padding_bytes: 100 * (mtu as u64),
            max_padding_frac: 0.0, // NOTE
            allowed_blocked_microsec: 0,
            max_blocking_frac: 0.0,
            states: vec![s0, s1],
            include_small_packets: true,
        };
        let m2 = m1.clone();

        // NOTE 0.5 max_padding_frac below
        let current_time = Instant::now();
        let mut f = Framework::new(vec![m1, m2], 0.5, 0.0, mtu, current_time).unwrap();

        // we have two machines that each can send 100 * mtu before their own or
        // any framework limits are applied (by design, see AllowedPaddingBytes)
        // trigger transition to get the loop going
        _ = f.trigger_events(
            &[TriggerEvent::NonPaddingRecv { bytes_recv: 0 }],
            current_time,
        );
        // we expect 100 padding actions per machine
        for _ in 0..100 {
            assert_eq!(
                f.actions[0],
                Some(Action::InjectPadding {
                    timeout: Duration::from_micros(2),
                    size: mtu as u16,
                    machine: MachineId(0),
                })
            );
            assert_eq!(
                f.actions[1],
                Some(Action::InjectPadding {
                    timeout: Duration::from_micros(2),
                    size: mtu as u16,
                    machine: MachineId(1),
                })
            );
            _ = f.trigger_events(
                &[
                    TriggerEvent::PaddingSent {
                        bytes_sent: mtu as u16,
                        machine: MachineId(0),
                    },
                    TriggerEvent::PaddingSent {
                        bytes_sent: mtu as u16,
                        machine: MachineId(1),
                    },
                ],
                current_time,
            );
        }

        // limit hit, last event should prevent the action and future actions
        assert_eq!(f.actions[0], None);
        assert_eq!(f.actions[1], None);
        _ = f.trigger_events(
            &[
                TriggerEvent::NonPaddingRecv {
                    bytes_recv: mtu as u16,
                },
                TriggerEvent::NonPaddingRecv {
                    bytes_recv: mtu as u16,
                },
            ],
            current_time,
        );
        assert_eq!(f.actions[0], None);
        assert_eq!(f.actions[1], None);

        // in sync?
        assert_eq!(f.runtime[0].padding_sent, f.runtime[1].padding_sent);
        assert_eq!(f.runtime[0].padding_sent, 100 * (mtu as u64));

        // OK, so we've sent in total 2*100*mtu of padding using two machines. This
        // means that we should need to send at least 2*100*mtu + 1 bytes before
        // padding is scheduled again
        for _ in 0..200 {
            assert_eq!(f.actions[0], None);
            assert_eq!(f.actions[1], None);
            _ = f.trigger_events(
                &[TriggerEvent::NonPaddingSent {
                    bytes_sent: mtu as u16,
                }],
                current_time,
            );
        }

        // the last byte should tip it over
        _ = f.trigger_events(
            &[TriggerEvent::NonPaddingSent { bytes_sent: 1 }],
            current_time,
        );
        assert_eq!(
            f.actions[0],
            Some(Action::InjectPadding {
                timeout: Duration::from_micros(2),
                size: mtu as u16,
                machine: MachineId(0),
            })
        );
        assert_eq!(
            f.actions[1],
            Some(Action::InjectPadding {
                timeout: Duration::from_micros(2),
                size: mtu as u16,
                machine: MachineId(1),
            })
        );
    }

    #[test]
    fn machine_max_blocking_frac() {
        // We create a machine that should be allowed to block for 10us before
        // machine limits are applied, then the machine should be limited from
        // blocking until after 10us, given the set max blocking fraction of
        // 0.5.
        let num_states = 2;
        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(1, 1.0);
        t.insert(Event::NonPaddingRecv, e);
        let s0 = State::new(t, num_states);
        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(1, 1.0);
        t.insert(Event::BlockingBegin, e.clone());
        t.insert(Event::BlockingEnd, e.clone());
        t.insert(Event::NonPaddingRecv, e.clone());
        let mut s1 = State::new(t, num_states);
        // block every 2us for 2us
        s1.timeout = Dist {
            dist: DistType::Uniform,
            param1: 2.0,
            param2: 2.0,
            start: 0.0,
            max: 0.0,
        };
        s1.action = Dist {
            dist: DistType::Uniform,
            param1: 2.0,
            param2: 2.0,
            start: 0.0,
            max: 0.0,
        };
        s1.action_is_block = true;

        let m = Machine {
            allowed_padding_bytes: 0,
            max_padding_frac: 0.0,
            allowed_blocked_microsec: 10, // NOTE
            max_blocking_frac: 0.5,       // NOTE
            states: vec![s0, s1],
            include_small_packets: false,
        };

        let mut current_time = Instant::now();
        let mut f = Framework::new(vec![m], 0.0, 0.0, 1500, current_time).unwrap();

        // trigger self to start the blocking (triggers action)
        _ = f.trigger_events(
            &[TriggerEvent::NonPaddingRecv { bytes_recv: 0 }],
            current_time,
        );

        // verify that we can block for 5*2=10us
        for _ in 0..5 {
            assert_eq!(
                f.actions[0],
                Some(Action::BlockOutgoing {
                    timeout: Duration::from_micros(2),
                    duration: Duration::from_micros(2),
                    overwrite: false,
                    machine: MachineId(0),
                })
            );

            _ = f.trigger_events(
                &[TriggerEvent::BlockingBegin {
                    machine: MachineId(0),
                }],
                current_time,
            );
            assert_eq!(
                f.actions[0],
                Some(Action::BlockOutgoing {
                    timeout: Duration::from_micros(2),
                    duration: Duration::from_micros(2),
                    overwrite: false,
                    machine: MachineId(0),
                })
            );
            current_time = current_time.add(Duration::from_micros(2));
            _ = f.trigger_events(&[TriggerEvent::BlockingEnd], current_time);
        }
        assert_eq!(f.actions[0], None);
        assert_eq!(f.runtime[0].blocking_duration, Duration::from_micros(10));

        // now we burn our allowed padding budget, should be blocked for 10us
        for _ in 0..5 {
            current_time = current_time.add(Duration::from_micros(2));
            _ = f.trigger_events(
                &[TriggerEvent::NonPaddingRecv { bytes_recv: 1000 }],
                current_time,
            );
            assert_eq!(f.actions[0], None);
        }
        assert_eq!(f.runtime[0].blocking_duration, Duration::from_micros(10));
        assert_eq!(
            current_time.duration_since(f.runtime[0].machine_start),
            Duration::from_micros(20)
        );

        // push over the limit, should be allowed
        current_time = current_time.add(Duration::from_micros(2));
        _ = f.trigger_events(
            &[TriggerEvent::NonPaddingRecv { bytes_recv: 1000 }],
            current_time,
        );
        assert_eq!(
            f.actions[0],
            Some(Action::BlockOutgoing {
                timeout: Duration::from_micros(2),
                duration: Duration::from_micros(2),
                overwrite: false,
                machine: MachineId(0),
            })
        );
    }

    #[test]
    fn framework_max_blocking_frac() {
        // We create a machine that should be allowed to block for 10us before
        // machine limits are applied, then the machine should be limited from
        // blocking until after 10us, given the set max blocking fraction of
        // 0.5 in the framework.
        let num_states = 2;
        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(1, 1.0);
        t.insert(Event::NonPaddingRecv, e);
        let s0 = State::new(t, num_states);
        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(1, 1.0);
        t.insert(Event::BlockingBegin, e.clone());
        t.insert(Event::BlockingEnd, e.clone());
        t.insert(Event::NonPaddingRecv, e.clone());
        let mut s1 = State::new(t, num_states);
        // block every 2us for 2us
        s1.timeout = Dist {
            dist: DistType::Uniform,
            param1: 2.0,
            param2: 2.0,
            start: 0.0,
            max: 0.0,
        };
        s1.action = Dist {
            dist: DistType::Uniform,
            param1: 2.0,
            param2: 2.0,
            start: 0.0,
            max: 0.0,
        };
        s1.action_is_block = true;

        let m = Machine {
            allowed_padding_bytes: 0,
            max_padding_frac: 0.0,
            allowed_blocked_microsec: 10, // NOTE
            max_blocking_frac: 0.0,       // NOTE, 0.0 here, 0.5 in framework below
            states: vec![s0, s1],
            include_small_packets: false,
        };

        let mut current_time = Instant::now();
        let mut f = Framework::new(vec![m], 0.0, 0.5, 1500, current_time).unwrap();

        // trigger self to start the blocking (triggers action)
        _ = f.trigger_events(
            &[TriggerEvent::NonPaddingRecv { bytes_recv: 0 }],
            current_time,
        );

        // verify that we can block for 5*2=10us
        for _ in 0..5 {
            assert_eq!(
                f.actions[0],
                Some(Action::BlockOutgoing {
                    timeout: Duration::from_micros(2),
                    duration: Duration::from_micros(2),
                    overwrite: false,
                    machine: MachineId(0),
                })
            );

            _ = f.trigger_events(
                &[TriggerEvent::BlockingBegin {
                    machine: MachineId(0),
                }],
                current_time,
            );
            assert_eq!(
                f.actions[0],
                Some(Action::BlockOutgoing {
                    timeout: Duration::from_micros(2),
                    duration: Duration::from_micros(2),
                    overwrite: false,
                    machine: MachineId(0),
                })
            );
            current_time = current_time.add(Duration::from_micros(2));
            _ = f.trigger_events(&[TriggerEvent::BlockingEnd], current_time);
        }
        assert_eq!(f.actions[0], None);
        assert_eq!(f.runtime[0].blocking_duration, Duration::from_micros(10));

        // now we burn our allowed padding budget, should be blocked for 10us
        for _ in 0..5 {
            current_time = current_time.add(Duration::from_micros(2));
            _ = f.trigger_events(
                &[TriggerEvent::NonPaddingRecv { bytes_recv: 1000 }],
                current_time,
            );
            assert_eq!(f.actions[0], None);
        }
        assert_eq!(f.runtime[0].blocking_duration, Duration::from_micros(10));
        assert_eq!(
            current_time.duration_since(f.runtime[0].machine_start),
            Duration::from_micros(20)
        );

        // push over the limit, should be allowed
        current_time = current_time.add(Duration::from_micros(2));
        _ = f.trigger_events(
            &[TriggerEvent::NonPaddingRecv { bytes_recv: 1000 }],
            current_time,
        );
        assert_eq!(
            f.actions[0],
            Some(Action::BlockOutgoing {
                timeout: Duration::from_micros(2),
                duration: Duration::from_micros(2),
                overwrite: false,
                machine: MachineId(0),
            })
        );
    }

    #[test]
    fn framework_machine_sampled_limit() {
        // we create a machine that samples a padding limit of 4 padding sent,
        // then should be prevented from padding further by transitioning to
        // self
        let num_states = 2;
        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(1, 1.0);
        t.insert(Event::NonPaddingSent, e);
        let s0 = State::new(t, num_states);
        let mut t: HashMap<Event, HashMap<usize, f64>> = HashMap::new();
        let mut e: HashMap<usize, f64> = HashMap::new();
        e.insert(1, 1.0);
        t.insert(Event::PaddingSent, e);
        let mut s1 = State::new(t, num_states);
        s1.timeout = Dist {
            dist: DistType::Uniform,
            param1: 1.0,
            param2: 1.0,
            start: 0.0,
            max: 0.0,
        };
        s1.limit = Dist {
            dist: DistType::Uniform,
            param1: 4.0,
            param2: 4.0,
            start: 0.0,
            max: 0.0,
        };

        let m = Machine {
            allowed_padding_bytes: 100000, // NOTE, will not apply
            max_padding_frac: 1.0,         // NOTE, will not apply
            allowed_blocked_microsec: 0,
            max_blocking_frac: 0.0,
            states: vec![s0, s1],
            include_small_packets: false,
        };

        let mut current_time = Instant::now();
        let mtu = 1500;
        let mut f = Framework::new(vec![m], 1.0, 0.0, mtu, current_time).unwrap();

        // trigger self to start the padding
        _ = f.trigger_events(
            &[TriggerEvent::NonPaddingSent { bytes_sent: 100 }],
            current_time,
        );

        assert_eq!(f.runtime[0].state_limit, 4);

        // verify that we can send 4 padding
        for _ in 0..4 {
            assert_eq!(
                f.actions[0],
                Some(Action::InjectPadding {
                    timeout: Duration::from_micros(1),
                    size: mtu as u16,
                    machine: MachineId(0),
                })
            );
            current_time = current_time.add(Duration::from_micros(1));
            _ = f.trigger_events(
                &[TriggerEvent::PaddingSent {
                    bytes_sent: mtu as u16,
                    machine: MachineId(0),
                }],
                current_time,
            );
        }

        // padding accounting correct
        assert_eq!(f.runtime[0].padding_sent, (mtu as u64) * 4);
        assert_eq!(f.runtime[0].nonpadding_sent, 100);

        // limit should be reached after 4 padding, blocking next action
        assert_eq!(f.actions[0], None);
        assert_eq!(f.runtime[0].state_limit, 0);
    }
}