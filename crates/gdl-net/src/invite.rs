//! Invite codes: the host's iroh endpoint address as a copy-pasteable
//! string, `GDL<protocol>-<base32>`.
//!
//! The base32 payload (RFC 4648 alphabet, no padding; lowercase, spaces
//! and line breaks are accepted when reading) is the endpoint id (32
//! bytes), the address count, each address — `0` + length + relay URL,
//! `4` + IPv4 + port, `6` + IPv6 + port — and a CRC-16 of all of it, so a
//! mangled paste is reported as such rather than dialled.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use iroh::{EndpointAddr, EndpointId, RelayUrl, TransportAddr};

use crate::PROTOCOL;

/// Why an invite code can't be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InviteError {
    /// Not an invite code at all.
    #[error("not a game invite (it should start with \"GDL\")")]
    NotAnInvite,
    /// Made by another version of the game.
    #[error("this invite is for network protocol {invite}; this build speaks protocol {ours}: both players need the same game version")]
    Version { invite: u16, ours: u16 },
    /// Damaged in copying.
    #[error("the invite code is damaged (copy it again, whole)")]
    Damaged,
}

/// Bytes kept of a relay URL (they're short: `https://…relay.n0.iroh.link./`).
const MAX_URL: usize = 255;

/// The invite code for `addr`.
pub(crate) fn encode(addr: &EndpointAddr) -> String {
    let mut b = Vec::with_capacity(96);
    b.extend_from_slice(addr.id.as_bytes());
    let addrs: Vec<&TransportAddr> = addr
        .addrs
        .iter()
        .filter(|a| matches!(a, TransportAddr::Ip(_)) || matches!(a, TransportAddr::Relay(u) if u.as_str().len() <= MAX_URL))
        .take(255)
        .collect();
    b.push(addrs.len() as u8);
    for a in addrs {
        match a {
            TransportAddr::Relay(url) => {
                b.push(0);
                b.push(url.as_str().len() as u8);
                b.extend_from_slice(url.as_str().as_bytes());
            }
            TransportAddr::Ip(SocketAddr::V4(s)) => {
                b.push(4);
                b.extend_from_slice(&s.ip().octets());
                b.extend_from_slice(&s.port().to_le_bytes());
            }
            TransportAddr::Ip(SocketAddr::V6(s)) => {
                b.push(6);
                b.extend_from_slice(&s.ip().octets());
                b.extend_from_slice(&s.port().to_le_bytes());
            }
            _ => {}
        }
    }
    let crc = crc16(&b);
    b.extend_from_slice(&crc.to_le_bytes());
    format!("GDL{PROTOCOL}-{}", base32(&b))
}

/// The endpoint address an invite code names.
pub(crate) fn decode(code: &str) -> Result<EndpointAddr, InviteError> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let rest = code.strip_prefix("GDL").or_else(|| code.strip_prefix("gdl")).ok_or(InviteError::NotAnInvite)?;
    // Written out without its dash, it's this protocol's.
    let ours = PROTOCOL.to_string();
    let (version, payload) = match rest.split_once('-') {
        Some(parts) => parts,
        None => (ours.as_str(), rest.strip_prefix(ours.as_str()).ok_or(InviteError::NotAnInvite)?),
    };
    let version: u16 = version.parse().map_err(|_| InviteError::NotAnInvite)?;
    if version != PROTOCOL {
        return Err(InviteError::Version { invite: version, ours: PROTOCOL });
    }
    let b = unbase32(payload).ok_or(InviteError::Damaged)?;
    if b.len() < 35 {
        return Err(InviteError::Damaged);
    }
    let (body, crc) = b.split_at(b.len() - 2);
    if crc16(body).to_le_bytes() != crc {
        return Err(InviteError::Damaged);
    }
    let id_bytes: [u8; 32] = body[..32].try_into().map_err(|_| InviteError::Damaged)?;
    let id = EndpointId::from_bytes(&id_bytes).map_err(|_| InviteError::Damaged)?;
    let mut at = 33;
    let mut addrs = Vec::new();
    let take = |at: &mut usize, n: usize| -> Result<&[u8], InviteError> {
        let s = body.get(*at..*at + n).ok_or(InviteError::Damaged)?;
        *at += n;
        Ok(s)
    };
    for _ in 0..body[32] {
        let kind = take(&mut at, 1)?[0];
        match kind {
            0 => {
                let n = usize::from(take(&mut at, 1)?[0]);
                let s = std::str::from_utf8(take(&mut at, n)?).map_err(|_| InviteError::Damaged)?;
                let url: RelayUrl = s.parse().map_err(|_| InviteError::Damaged)?;
                addrs.push(TransportAddr::Relay(url));
            }
            4 => {
                let ip: [u8; 4] = take(&mut at, 4)?.try_into().map_err(|_| InviteError::Damaged)?;
                let port = u16::from_le_bytes(take(&mut at, 2)?.try_into().map_err(|_| InviteError::Damaged)?);
                addrs.push(TransportAddr::Ip(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), port)));
            }
            6 => {
                let ip: [u8; 16] = take(&mut at, 16)?.try_into().map_err(|_| InviteError::Damaged)?;
                let port = u16::from_le_bytes(take(&mut at, 2)?.try_into().map_err(|_| InviteError::Damaged)?);
                addrs.push(TransportAddr::Ip(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(ip)), port)));
            }
            _ => return Err(InviteError::Damaged),
        }
    }
    if at != body.len() {
        return Err(InviteError::Damaged);
    }
    Ok(EndpointAddr::from_parts(id, addrs))
}

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len() * 8 / 5 + 1);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &x in b {
        acc = (acc << 8) | u32::from(x);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn unbase32(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 5 / 8);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = ALPHABET.iter().position(|&a| a == c.to_ascii_uppercase())? as u32;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
        acc &= (1 << bits) - 1;
    }
    Some(out)
}

/// CRC-16/CCITT-FALSE.
fn crc16(b: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &x in b {
        crc ^= u16::from(x) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr() -> EndpointAddr {
        let id = iroh::SecretKey::from_bytes(&[7; 32]).public();
        EndpointAddr::from_parts(
            id,
            [
                TransportAddr::Relay("https://euw1-1.relay.n0.iroh.link./".parse().unwrap()),
                TransportAddr::Ip("192.168.1.20:51234".parse().unwrap()),
                TransportAddr::Ip("[2001:db8::7]:51234".parse().unwrap()),
            ],
        )
    }

    #[test]
    fn codes_come_back_and_survive_casual_pasting() {
        let a = addr();
        let code = encode(&a);
        assert!(code.starts_with(&format!("GDL{PROTOCOL}-")), "{code}");
        assert_eq!(decode(&code), Ok(a.clone()));
        // Lowercase, wrapped over lines, with spaces round it.
        let messy = format!("  {}\n{} ", code[..20].to_lowercase(), &code[20..]);
        assert_eq!(decode(&messy), Ok(a.clone()));
        // Written out with a space for its dash (the game's font has none).
        assert_eq!(decode(&code.replacen('-', " ", 1)), Ok(a));
        assert_eq!(base32(&[]), "");
        assert_eq!(unbase32(&base32(&[1, 2, 3, 4, 5, 6])), Some(vec![1, 2, 3, 4, 5, 6]));
    }

    #[test]
    fn bad_codes_say_why() {
        let code = encode(&addr());
        assert_eq!(decode("hello"), Err(InviteError::NotAnInvite));
        let other = code.replacen(&format!("GDL{PROTOCOL}-"), &format!("GDL{}-", PROTOCOL + 1), 1);
        assert_eq!(decode(&other), Err(InviteError::Version { invite: PROTOCOL + 1, ours: PROTOCOL }));
        // A character changed, or the end cut off.
        let mut changed = code.clone().into_bytes();
        let k = changed.len() - 10;
        changed[k] = if changed[k] == b'A' { b'B' } else { b'A' };
        assert_eq!(decode(&String::from_utf8(changed).unwrap()), Err(InviteError::Damaged));
        assert_eq!(decode(&code[..code.len() - 4]), Err(InviteError::Damaged));
        assert_eq!(decode(&format!("{code}!")), Err(InviteError::Damaged));
    }
}
