# Playing Gauntlet: Dark Legacy (Rust engine)

## What you need

- This program for your system (`gdl-game`, or `gdl-game.exe` on Windows),
  from the [Releases page](https://github.com/dmonskull/gauntlet-rust/releases).
  On Linux it needs the usual desktop libraries (ALSA, udev, GTK 3).
- **Your own copy of Gauntlet: Dark Legacy for the GameCube** (USA,
  `GUNE5D`): the disc image (`.iso`, `.gcm` or Dolphin's `.rvz`), or an
  extracted disc folder. The program ships no game data.

## Start

Unzip the download and run the program: double-click it, or in a terminal
in its folder type `./gdl-game` (`gdl-game.exe` on Windows; the `./` is
needed on macOS and Linux). The first time, a file picker asks for your
game; pick the disc image or folder. It's remembered — next time the game
starts straight away (`./gdl-game --forget` asks again).

On macOS the first run may say Apple can't check the app (it isn't signed):
in the program's folder run `xattr -d com.apple.quarantine gdl-game` once,
or allow it under System Settings → Privacy & Security.

## Controls

Keyboard and mouse, or any game pad (plug in more pads for local co-op).
Enter / Start pauses; the Settings menu has the controls and lets you
rebind keys.

## Local co-op (one machine)

Title → Start → **Local Game**. Each player presses Start (or A) on their
own pad on the select screen to join; more players can join later in the
tower (Manage Character) — as in the original.

## Online co-op (up to four machines)

No ports to open: the game connects the machines itself (through public
relays when it has to).

1. **Host:** Title → Start → **Online Game** → **Host Game**. After a few
   seconds the select screen opens and the **invite code is copied** to your
   clipboard. Send it to your friends (chat, e-mail…). Press C to copy it
   again.
2. **Join:** copy the invite code your friend sent you, then Title → Start →
   **Online Game** → **Join Game**.
3. Everyone picks a hero on their own column: **New**, or **Load** a hero
   you saved on your machine.
4. When everyone is ready the host presses **Start**.

Friends can **join later** with the same code (Start → **Invite** copies it
again): they pick their hero and come in when the party is next in the
tower.

Each player has their own screen and their own settings; the host picks the
camera (Start → **Camera**: each player's own, or the overhead co-op one).
If your hero dies you watch a teammate (L / R picks another) until the
level ends. Start opens the online menu — the game goes on underneath. In
the tower it also has **Shop** and **Inventory** (they open for everyone)
and **Manage Character**: change or load your hero (everyone starts again
in the tower with it), or **Save** it on your machine.

Everyone must run **the same version** of the program and the same game
(USA disc); Windows, Mac and Linux players can play together. If the
machines ever disagree, the level starts again for everyone, each hero with
what they had.
