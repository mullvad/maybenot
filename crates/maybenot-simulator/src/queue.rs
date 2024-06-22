//! The main queue of events in the simulator.

use std::{
    collections::BinaryHeap,
    time::{Duration, Instant},
};

use maybenot::event::TriggerEvent;
use rand::RngCore;

use crate::SimEvent;

/// SimQueue represents the queue of events that are to be processed by the
/// simulator. It is a wrapper around an EventQueue for the client and an
/// EventQueue for the server. The goal is to never have to search through
/// any of the queues, but to be able to directly access the next event
/// that is to be processed with as little work as possible.
#[derive(Debug, Clone)]
pub struct SimQueue {
    client: EventQueue,
    server: EventQueue,
}

impl Default for SimQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl SimQueue {
    pub fn new() -> SimQueue {
        SimQueue {
            client: EventQueue::new(),
            server: EventQueue::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.client.len() + self.server.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push<R: RngCore>(
        &mut self,
        event: TriggerEvent,
        is_client: bool,
        contains_padding: bool,
        time: Instant,
        delay: Duration,
        rng: &mut R,
    ) {
        self.push_sim(SimEvent {
            event,
            time,
            delay,
            client: is_client,
            contains_padding,
            bypass: false,
            replace: false,
            fuzz: rng.next_u64(),
        });
    }

    pub fn push_sim(&mut self, item: SimEvent) {
        match item.client {
            true => self.client.push(item),
            false => self.server.push(item),
        }
    }

    pub fn peek(&self) -> Option<&SimEvent> {
        match self.len() {
            0 => None,
            _ => {
                // peak all, per def, it's one of them
                let c = self.client.peek();
                let s = self.server.peek();

                if c > s {
                    c
                } else {
                    s
                }
            }
        }
    }

    pub fn remove(&mut self, item: &SimEvent) {
        match item.client {
            true => self.client.remove(item),
            false => self.server.remove(item),
        }
    }

    pub fn peek_blocking(&self, blocking_bypassable: bool, is_client: bool) -> Option<&SimEvent> {
        match is_client {
            true => peak_blocking(&self.client, blocking_bypassable),
            false => peak_blocking(&self.server, blocking_bypassable),
        }
    }

    pub fn peek_non_blocking(
        &self,
        blocking_bypassable: bool,
        is_client: bool,
    ) -> Option<&SimEvent> {
        match is_client {
            true => peak_non_blocking(&self.client, blocking_bypassable),
            false => peak_non_blocking(&self.server, blocking_bypassable),
        }
    }
}

fn peak_blocking(queue: &EventQueue, blocking_bypassable: bool) -> Option<&SimEvent> {
    if blocking_bypassable {
        // only blocking events are then blocking
        queue.peek_blocking()
    } else {
        // if the current blocking is not bypassable, then we need to
        // consider blocking_bypassable events as also blocking
        let b = queue.peek_blocking();
        let bb = queue.peek_blocking_bypassable();

        if b > bb {
            b
        } else {
            bb
        }
    }
}

fn peak_non_blocking(queue: &EventQueue, blocking_bypassable: bool) -> Option<&SimEvent> {
    if blocking_bypassable {
        // if the current blocking is not bypassable, then we need to
        // consider blocking_bypassable as a non_blocking event
        let bb = queue.peek_blocking_bypassable();
        let n = queue.peek_non_blocking();

        if bb > n {
            bb
        } else {
            n
        }
    } else {
        // only non_blocking events are then non_blocking
        queue.peek_non_blocking()
    }
}

/// EventQueue represents the queue of events that are waiting to be processed
/// in order (time-wise). The queue is split into three parts:
/// 1. blocking: events that are blocking, i.e., that must take blocking into
///    account.
/// 2. blocking_bypassable: events that are blocking, but that MAY be bypassed
///    (depending on the type of active blocking).
/// 3. non_blocking: events that are always not blocking.
#[derive(Debug, Clone)]
struct EventQueue {
    blocking: BinaryHeap<SimEvent>,
    blocking_bypassable: BinaryHeap<SimEvent>,
    non_blocking: BinaryHeap<SimEvent>,
}

impl EventQueue {
    pub fn new() -> EventQueue {
        EventQueue {
            blocking: BinaryHeap::with_capacity(1024),
            blocking_bypassable: BinaryHeap::with_capacity(1024),
            non_blocking: BinaryHeap::with_capacity(5000),
        }
    }

    pub fn len(&self) -> usize {
        self.blocking.len() + self.blocking_bypassable.len() + self.non_blocking.len()
    }

    pub fn push(&mut self, item: SimEvent) {
        match item.event {
            TriggerEvent::TunnelSent => {
                match item.bypass {
                    true => self.blocking_bypassable.push(item),
                    false => self.blocking.push(item),
                };
            }
            _ => {
                self.non_blocking.push(item);
            }
        }
    }

    pub fn peek(&self) -> Option<&SimEvent> {
        match self.len() {
            0 => None,
            _ => {
                // peak all, per def, it's one of them
                let b = self.blocking.peek();
                let bb = self.blocking_bypassable.peek();
                let n = self.non_blocking.peek();

                // is b first?
                if b > bb && b > n {
                    b
                // is bb first?
                } else if bb > n {
                    bb
                // has to be n then
                } else {
                    n
                }
            }
        }
    }

    pub fn remove(&mut self, item: &SimEvent) {
        match item.event {
            TriggerEvent::TunnelSent => {
                match item.bypass {
                    true => self.blocking_bypassable.pop(),
                    false => self.blocking.pop(),
                };
            }
            _ => {
                self.non_blocking.pop();
            }
        }
    }

    pub fn peek_blocking(&self) -> Option<&SimEvent> {
        self.blocking.peek()
    }

    pub fn peek_blocking_bypassable(&self) -> Option<&SimEvent> {
        self.blocking_bypassable.peek()
    }

    pub fn peek_non_blocking(&self) -> Option<&SimEvent> {
        self.non_blocking.peek()
    }
}
