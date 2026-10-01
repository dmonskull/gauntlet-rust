//! Host and client in one process over real iroh connections: loopback
//! only, relays off, so it needs no internet. The public-relay test is
//! `#[ignore]`d (run it with `cargo test -p gdl-net -- --ignored`).

use std::time::{Duration, Instant};

use gdl_net::{Bundle, NetConfig, NetEvent, NetSession, PlayerInput, Relays, Target, Tick};

fn local(name: &str) -> NetConfig {
    NetConfig {
        name: name.into(),
        relays: Relays::Disabled,
        bind: Some("127.0.0.1:0".parse().unwrap()),
        game_version: "test build".into(),
        auto_delay: false,
        ..NetConfig::default()
    }
}

/// Polls `f` until it holds or `secs` pass.
fn wait_until(secs: u64, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn input(slot: u8, tick: Tick) -> PlayerInput {
    PlayerInput { stick: [slot as i8, (tick % 100) as i8], c_stick: [1, -1], buttons: tick }
}

/// Runs both sessions' games until each has simulated `ticks` ticks.
fn play(sessions: &[&NetSession], ticks: usize) -> Vec<Vec<Bundle>> {
    let mut ran = vec![Vec::new(); sessions.len()];
    let done = wait_until(30, || {
        for (k, s) in sessions.iter().enumerate() {
            let tick = ran[k].len() as Tick;
            for slot in s.local_slots() {
                s.set_local_input(slot, input(slot, tick + Tick::from(s.input_delay())));
            }
            if let Some(b) = s.ready_inputs(tick) {
                ran[k].push(b);
                s.report_checksum(tick, tick as u64 * 7);
            }
        }
        ran.iter().all(|r| r.len() >= ticks)
    });
    assert!(done, "only got {:?} ticks", ran.iter().map(Vec::len).collect::<Vec<_>>());
    ran
}

#[test]
fn a_friend_joins_with_the_invite_and_both_play_the_same_ticks() {
    let (host, invite) = NetSession::host(local("Host")).expect("host");
    assert!(invite.starts_with(&format!("GDL{}-", gdl_net::PROTOCOL)), "{invite}");
    let client = NetSession::join(&invite, local("Guest")).expect("join");
    let (mut host_events, mut client_events) = (Vec::new(), Vec::new());
    let joined = wait_until(15, || {
        host_events.extend(host.poll_events());
        client_events.extend(client.poll_events());
        client_events.iter().any(|e| matches!(e, NetEvent::Connected { .. }))
    });
    assert!(joined, "client events {client_events:?}");
    assert!(client_events.contains(&NetEvent::Connected { you: 1, slots: vec![1], late: false }), "{client_events:?}");
    assert!(host_events.iter().any(|e| matches!(e, NetEvent::PeerJoined { peer: 1, name, .. } if name == "Guest")), "{host_events:?}");
    assert_eq!(client.roster().len(), 2);

    host.start().expect("start");
    let ran = play(&[&host, &client], 120);
    let n = ran[0].len().min(ran[1].len());
    assert_eq!(ran[0][..n], ran[1][..n]);
    for (t, b) in ran[0].iter().enumerate().skip(3) {
        assert_eq!(b[0], Some(input(0, t as Tick)));
        assert_eq!(b[1], Some(input(1, t as Tick)));
        assert_eq!((b[2], b[3]), (None, None));
    }

    // A control message each way.
    client.send_control(Target::Host, b"ready".to_vec());
    host.send_control(Target::All, b"go".to_vec());
    let mut host_got = Vec::new();
    let mut client_got = Vec::new();
    let delivered = wait_until(10, || {
        for e in host.poll_events() {
            if let NetEvent::Control { from, bytes } = e {
                host_got.push((from, bytes));
            }
        }
        for e in client.poll_events() {
            if let NetEvent::Control { from, bytes } = e {
                client_got.push((from, bytes));
            }
        }
        !host_got.is_empty() && !client_got.is_empty()
    });
    assert!(delivered);
    assert_eq!(host_got, vec![(1, b"ready".to_vec())]);
    assert_eq!(client_got, vec![(0, b"go".to_vec())]);
    assert!(client.rtt().is_some() && host.rtt().is_some());

    // The friend leaves: the host hears it and carries on alone.
    client.leave();
    let mut left = false;
    assert!(wait_until(10, || {
        left |= host.poll_events().iter().any(|e| matches!(e, NetEvent::PeerLeft { peer: 1, .. }));
        left
    }));
    let mut tick = ran[0].len() as Tick;
    assert!(wait_until(10, || {
        host.set_local_input(0, input(0, tick + 3));
        match host.ready_inputs(tick) {
            Some(b) => {
                tick += 1;
                b[1].is_none()
            }
            None => false,
        }
    }));
}

#[test]
fn mismatched_builds_and_versions_are_refused() {
    let (host, invite) = NetSession::host(local("Host")).expect("host");
    let other = NetConfig { game_version: "another build".into(), ..local("Guest") };
    let client = NetSession::join(&invite, other).expect("join");
    let mut events = Vec::new();
    assert!(wait_until(15, || {
        events.extend(client.poll_events());
        events.iter().any(|e| matches!(e, NetEvent::Failed(_)))
    }));
    assert!(events.iter().any(|e| matches!(e, NetEvent::Failed(r) if r.contains("build"))), "{events:?}");
    // An invite from another protocol version says so before dialling.
    let next = gdl_net::PROTOCOL + 1;
    let wrong = invite.replacen(&format!("GDL{}-", gdl_net::PROTOCOL), &format!("GDL{next}-"), 1);
    let err = NetSession::join(&wrong, local("Guest")).err().expect("refused");
    assert!(err.to_string().contains(&format!("protocol {next}")), "{err}");
    drop(host);
}

#[test]
#[ignore = "needs the public internet and n0's relays"]
fn joins_through_the_public_relays() {
    let public = |name: &str| NetConfig { name: name.into(), game_version: "test build".into(), ..NetConfig::default() };
    let (host, invite) = NetSession::host(public("Host")).expect("host");
    let client = NetSession::join(&invite, public("Guest")).expect("join");
    let mut events = Vec::new();
    assert!(wait_until(30, || {
        events.extend(client.poll_events());
        events.iter().any(|e| matches!(e, NetEvent::Connected { .. } | NetEvent::Failed(_)))
    }));
    assert!(events.iter().any(|e| matches!(e, NetEvent::Connected { .. })), "{events:?}");
    // The short code, found through n0's address lookup.
    let short = host.short_invite().expect("a short code");
    assert!(short.len() < invite.len() && short.len() < 70, "{short}");
    let second = NetSession::join(&short, public("Second")).expect("join");
    let mut events = Vec::new();
    assert!(wait_until(30, || {
        events.extend(second.poll_events());
        events.iter().any(|e| matches!(e, NetEvent::Connected { .. } | NetEvent::Failed(_)))
    }));
    assert!(events.iter().any(|e| matches!(e, NetEvent::Connected { .. })), "{events:?}");
    host.start().expect("start");
    let ran = play(&[&host, &client, &second], 60);
    let n = ran.iter().map(Vec::len).min().unwrap_or(0);
    assert_eq!(ran[0][..n], ran[1][..n]);
    assert_eq!(ran[0][..n], ran[2][..n]);
}
