# Playing Gauntlet: Dark Legacy (Rust engine)

## What you need

- This program for your system (`gdl-game`, or `gdl-game.exe` on Windows),
  from the [Releases page](https://github.com/dmonskull/gauntlet-rust/releases).
  On Linux it needs the usual desktop libraries (ALSA, udev, GTK 3).
- **Your own copy of Gauntlet: Dark Legacy for the GameCube** (USA,
  `GUNE5D`): the disc image (`.iso`, `.gcm` or Dolphin's `.rvz`), or an
  extracted disc folder. The program ships no game data.

## Start

Run the program. The first time, a file picker asks for your game; pick the
disc image or folder. It's remembered — next time the game starts straight
away (`gdl-game --forget` asks again).

On macOS, if it says the app can't be checked: right-click → Open once, or
run `xattr -d com.apple.quarantine gdl-game` in its folder.

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

Each player has their own screen and camera and their own settings. If your
hero dies you watch a teammate (L / R picks another) until the level ends.
Start opens the online menu (Settings, Leave Game) — the game goes on
underneath. Saving: your hero is yours — save it from a local game.

Everyone must run **the same version** of the program and the same game
(USA disc). Machines of different kinds (say a Mac and a Windows PC) can
drift apart; the game shows "Out of sync" if they do.
