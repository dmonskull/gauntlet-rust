//! The protocol's messages and their byte encoding (little-endian, no
//! padding). Reliable messages travel as frames on the connection's one
//! bidirectional stream (`u32` length, then the message); the lockstep
//! traffic travels as QUIC datagrams, one message each.

use crate::{Bundle, MAX_SLOTS, PeerId, PeerInfo, PlayerInput, Tick};

/// A protocol message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Msg {
    // --- Reliable (the stream) ---
    /// Client → host, first thing on the stream: who it is and how many
    /// players it brings.
    Hello { protocol: u16, game: String, name: String, players: u8 },
    /// Host → client: its peer id, the input delay, whether the game is
    /// under way (`late`: its slots join at the next restart), and everyone
    /// in the game (itself included).
    Welcome { you: PeerId, delay: u8, late: bool, roster: Vec<PeerInfo> },
    /// Host → client: refused (full, started, wrong build); the host
    /// closes the connection after it.
    Reject { reason: String },
    /// Host → clients: someone joined.
    Joined(PeerInfo),
    /// Host → clients: someone left; their slots are empty from tick
    /// `from` on (when the game had started).
    Left { peer: PeerId, from: Option<Tick> },
    /// Host → clients: lockstep starts at tick 0.
    Start { delay: u8 },
    /// Host → clients: lockstep starts again at tick 0 as `epoch`, every
    /// machine in the roster playing (late joiners too); what was under way
    /// is forgotten.
    Restart { delay: u8, epoch: u8 },
    /// Host → clients: schedule local inputs this many ticks ahead from
    /// now on (only ever raised).
    Delay { delay: u8 },
    /// A game message. Client → host: `peer` is the target code
    /// ([`crate::Target`]); host → client: `peer` is the sender.
    Control { peer: u8, bytes: Vec<u8> },
    /// Client → host: its state checksum at `tick` of `epoch`.
    Checksum { epoch: u8, tick: Tick, value: u64 },
    /// Host → clients: the checksums for `tick` disagree.
    Desync { tick: Tick },
    /// Either way: leaving for good.
    Leave,

    // --- Unreliable (datagrams) ---
    /// Client → host: its slots' inputs for ticks `base..` (each tick
    /// `slots` inputs, in slot order), repeated until acknowledged; `ack`
    /// is the first tick whose bundle it still lacks.
    Inputs { epoch: u8, ack: Tick, base: Tick, slots: u8, inputs: Vec<PlayerInput> },
    /// Host → client: the bundles for ticks `base..`, repeated until
    /// acknowledged; `ack` is the first tick the host still lacks this
    /// client's inputs for; `waiting` the peers (bit per peer id < 8) the
    /// host lacks inputs from for its next bundle.
    Bundles { epoch: u8, ack: Tick, base: Tick, waiting: u8, bundles: Vec<Bundle> },
    /// Round-trip probes (and keep-alives).
    Ping { id: u32 },
    Pong { id: u32 },
}

impl Msg {
    /// Whether it must go on the reliable stream.
    pub(crate) fn reliable(&self) -> bool {
        !matches!(self, Msg::Inputs { .. } | Msg::Bundles { .. } | Msg::Ping { .. } | Msg::Pong { .. })
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut w = Writer(Vec::with_capacity(64));
        match self {
            Msg::Hello { protocol, game, name, players } => {
                w.u8(1);
                w.u16(*protocol);
                w.str(game);
                w.str(name);
                w.u8(*players);
            }
            Msg::Welcome { you, delay, late, roster } => {
                w.u8(2);
                w.u8(*you);
                w.u8(*delay);
                w.u8(u8::from(*late));
                w.u8(roster.len() as u8);
                for p in roster {
                    w.peer(p);
                }
            }
            Msg::Reject { reason } => {
                w.u8(3);
                w.str(reason);
            }
            Msg::Joined(p) => {
                w.u8(4);
                w.peer(p);
            }
            Msg::Left { peer, from } => {
                w.u8(5);
                w.u8(*peer);
                match from {
                    Some(t) => {
                        w.u8(1);
                        w.u32(*t);
                    }
                    None => w.u8(0),
                }
            }
            Msg::Start { delay } => {
                w.u8(6);
                w.u8(*delay);
            }
            Msg::Restart { delay, epoch } => {
                w.u8(12);
                w.u8(*delay);
                w.u8(*epoch);
            }
            Msg::Delay { delay } => {
                w.u8(7);
                w.u8(*delay);
            }
            Msg::Control { peer, bytes } => {
                w.u8(8);
                w.u8(*peer);
                w.u32(bytes.len() as u32);
                w.0.extend_from_slice(bytes);
            }
            Msg::Checksum { epoch, tick, value } => {
                w.u8(9);
                w.u8(*epoch);
                w.u32(*tick);
                w.u64(*value);
            }
            Msg::Desync { tick } => {
                w.u8(10);
                w.u32(*tick);
            }
            Msg::Leave => w.u8(11),
            Msg::Inputs { epoch, ack, base, slots, inputs } => {
                w.u8(20);
                w.u8(*epoch);
                w.u32(*ack);
                w.u32(*base);
                w.u8(*slots);
                w.u8((inputs.len() / usize::from((*slots).max(1))) as u8);
                for i in inputs {
                    w.input(i);
                }
            }
            Msg::Bundles { epoch, ack, base, waiting, bundles } => {
                w.u8(21);
                w.u8(*epoch);
                w.u32(*ack);
                w.u32(*base);
                w.u8(*waiting);
                w.u8(bundles.len() as u8);
                for b in bundles {
                    let mask = b.iter().enumerate().fold(0u8, |m, (k, i)| if i.is_some() { m | 1 << k } else { m });
                    w.u8(mask);
                    for i in b.iter().flatten() {
                        w.input(i);
                    }
                }
            }
            Msg::Ping { id } => {
                w.u8(22);
                w.u32(*id);
            }
            Msg::Pong { id } => {
                w.u8(23);
                w.u32(*id);
            }
        }
        w.0
    }

    pub(crate) fn decode(bytes: &[u8]) -> Option<Msg> {
        let mut r = Reader { b: bytes, at: 0 };
        let msg = match r.u8()? {
            1 => Msg::Hello { protocol: r.u16()?, game: r.str()?, name: r.str()?, players: r.u8()? },
            2 => {
                let (you, delay, late, n) = (r.u8()?, r.u8()?, r.u8()? != 0, r.u8()?);
                let roster = (0..n).map(|_| r.peer()).collect::<Option<Vec<_>>>()?;
                Msg::Welcome { you, delay, late, roster }
            }
            3 => Msg::Reject { reason: r.str()? },
            4 => Msg::Joined(r.peer()?),
            5 => {
                let peer = r.u8()?;
                let from = match r.u8()? {
                    0 => None,
                    _ => Some(r.u32()?),
                };
                Msg::Left { peer, from }
            }
            6 => Msg::Start { delay: r.u8()? },
            12 => Msg::Restart { delay: r.u8()?, epoch: r.u8()? },
            7 => Msg::Delay { delay: r.u8()? },
            8 => {
                let peer = r.u8()?;
                let n = r.u32()? as usize;
                Msg::Control { peer, bytes: r.bytes(n)?.to_vec() }
            }
            9 => Msg::Checksum { epoch: r.u8()?, tick: r.u32()?, value: r.u64()? },
            10 => Msg::Desync { tick: r.u32()? },
            11 => Msg::Leave,
            20 => {
                let (epoch, ack, base, slots, count) = (r.u8()?, r.u32()?, r.u32()?, r.u8()?, r.u8()?);
                if usize::from(slots) > MAX_SLOTS {
                    return None;
                }
                let inputs = (0..usize::from(slots) * usize::from(count)).map(|_| r.input()).collect::<Option<Vec<_>>>()?;
                Msg::Inputs { epoch, ack, base, slots, inputs }
            }
            21 => {
                let (epoch, ack, base, waiting, count) = (r.u8()?, r.u32()?, r.u32()?, r.u8()?, r.u8()?);
                let mut bundles = Vec::with_capacity(usize::from(count));
                for _ in 0..count {
                    let mask = r.u8()?;
                    let mut b: Bundle = [None; MAX_SLOTS];
                    for (k, slot) in b.iter_mut().enumerate() {
                        if mask & 1 << k != 0 {
                            *slot = Some(r.input()?);
                        }
                    }
                    bundles.push(b);
                }
                Msg::Bundles { epoch, ack, base, waiting, bundles }
            }
            22 => Msg::Ping { id: r.u32()? },
            23 => Msg::Pong { id: r.u32()? },
            _ => return None,
        };
        // Trailing bytes mean a different encoding: refuse it.
        (r.at == bytes.len()).then_some(msg)
    }
}

/// Bytes one bundle takes at most (its mask and four inputs).
pub(crate) const BUNDLE_BYTES: usize = 1 + MAX_SLOTS * INPUT_BYTES;
/// Bytes one input takes.
pub(crate) const INPUT_BYTES: usize = 8;
/// Bytes of an `Inputs` or `Bundles` header.
pub(crate) const HEADER_BYTES: usize = 12;

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        let b = &s.as_bytes()[..s.len().min(u16::MAX as usize)];
        self.u16(b.len() as u16);
        self.0.extend_from_slice(b);
    }
    fn input(&mut self, i: &PlayerInput) {
        self.0.extend_from_slice(&[i.stick[0] as u8, i.stick[1] as u8, i.c_stick[0] as u8, i.c_stick[1] as u8]);
        self.u32(i.buttons);
    }
    fn peer(&mut self, p: &PeerInfo) {
        self.u8(p.peer);
        self.str(&p.name);
        self.u8(p.slots.len() as u8);
        self.0.extend_from_slice(&p.slots);
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn bytes(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.bytes(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.bytes(8)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<String> {
        let n = usize::from(self.u16()?);
        String::from_utf8(self.bytes(n)?.to_vec()).ok()
    }
    fn input(&mut self) -> Option<PlayerInput> {
        let b = self.bytes(4)?;
        let (stick, c_stick) = ([b[0] as i8, b[1] as i8], [b[2] as i8, b[3] as i8]);
        Some(PlayerInput { stick, c_stick, buttons: self.u32()? })
    }
    fn peer(&mut self) -> Option<PeerInfo> {
        let peer = self.u8()?;
        let name = self.str()?;
        let n = usize::from(self.u8()?);
        let slots = self.bytes(n)?.to_vec();
        if slots.iter().any(|&s| usize::from(s) >= MAX_SLOTS) {
            return None;
        }
        Some(PeerInfo { peer, name, slots })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(k: i8) -> PlayerInput {
        PlayerInput { stick: [k, -k], c_stick: [-128, 127], buttons: 0xDEAD_BEEF ^ k as u32 }
    }

    #[test]
    fn every_message_comes_back() {
        let roster = vec![
            PeerInfo { peer: 0, name: "Host".into(), slots: vec![0] },
            PeerInfo { peer: 1, name: "Ünïcode".into(), slots: vec![1, 2] },
        ];
        let msgs = vec![
            Msg::Hello { protocol: 1, game: "build 7".into(), name: "Ann".into(), players: 2 },
            Msg::Welcome { you: 1, delay: 3, late: true, roster: roster.clone() },
            Msg::Reject { reason: "full".into() },
            Msg::Joined(roster[1].clone()),
            Msg::Left { peer: 2, from: Some(400) },
            Msg::Left { peer: 2, from: None },
            Msg::Start { delay: 4 },
            Msg::Restart { delay: 4, epoch: 3 },
            Msg::Delay { delay: 6 },
            Msg::Control { peer: 0xFF, bytes: vec![1, 2, 3] },
            Msg::Checksum { epoch: 2, tick: 90, value: u64::MAX - 5 },
            Msg::Desync { tick: 90 },
            Msg::Leave,
            Msg::Inputs { epoch: 1, ack: 10, base: 12, slots: 2, inputs: vec![input(1), input(2), input(3), input(4)] },
            Msg::Bundles { epoch: 1, ack: 15, base: 9, waiting: 0b100, bundles: vec![[Some(input(5)), None, Some(input(6)), None], [None; 4]] },
            Msg::Ping { id: 77 },
            Msg::Pong { id: 77 },
        ];
        for m in msgs {
            let bytes = m.encode();
            assert_eq!(Msg::decode(&bytes).as_ref(), Some(&m), "{m:?}");
            // Cut short or padded, it's refused.
            assert_eq!(Msg::decode(&bytes[..bytes.len() - 1]), None, "{m:?}");
            let mut longer = bytes.clone();
            longer.push(0);
            assert_eq!(Msg::decode(&longer), None, "{m:?}");
        }
        assert_eq!(Msg::decode(&[99]), None);
    }

    #[test]
    fn header_and_bundle_sizes_match_the_encoding() {
        let full = [Some(input(1)); MAX_SLOTS];
        let m = Msg::Bundles { epoch: 0, ack: 0, base: 0, waiting: 0, bundles: vec![full; 3] };
        assert_eq!(m.encode().len(), HEADER_BYTES + 3 * BUNDLE_BYTES);
        let m = Msg::Inputs { epoch: 0, ack: 0, base: 0, slots: 2, inputs: vec![input(0); 6] };
        assert_eq!(m.encode().len(), HEADER_BYTES + 6 * INPUT_BYTES);
    }
}
