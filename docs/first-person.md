# Optional first-person views

Start → Settings → Controls → First Person View → On. Each local player
uses their own saved option; online each machine uses its player 1 option
for its own hero, regardless of the hero's network slot or the host's
shared-camera choice. Off restores the existing camera and control style.

## Controls and combat

Only inputs carrying FIRST_PERSON use mouse/right-stick look, movement
relative to the gaze, and gaze-directed throws. Classic movement remains
relative to the original camera, including the Robotron C-stick attack
scheme. Action bindings stay the same. First-person side/back movement
uses the existing strafe actions; forward movement uses normal walk/run.
Speeds, collision, action factors, attack timing, damage, turbo costs,
potions and powers use the existing gameplay systems.

Mouse pixels accumulate between rendered frames and are consumed once
per 30 Hz tick, or once when the online session commits a future input.
Waiting for a network bundle retains the pixels. Large mouse movements
retain their remainder rather than clipping away the turn. The input
word carries the mode and look stick, and the state hash includes the
look angles. Lockstep revision 2 excludes older builds with different
movement rules. Controller look is 2.4 radians/s; mouse sensitivity is
0.003 radians/pixel. The gaze is interpolated for rendering.

## Equipment and screens

The view draws the hero's own live forearm, hand and equipped weapon
meshes, materials, colours and animation pose. Power weapon swaps are
copied from the wrist descendants. A separate transparent render target
keeps the equipment from cutting through level walls. Forearms extend
from off-screen shoulders to their animated wrists; the full transform
is retained so its stretch does not lose shear. This is presentation
only, with no collision or combat entities. A small central marker shows
the throw direction. The camera stays steady; the original arm poses
supply the movement. Eye height follows the hero's class and growth.

Solo and online views use the full window. If any local co-op player
chooses first-person, two local heroes use left/right panes and three or
four use a grid. Each pane may be first-person or classic. Equipment
moves further from the lens in narrow panes, and status panels fit their
own panes. Opening shots, cut scenes, going out and menus keep the
original presentation. Menus and window focus loss release mouse capture.

## Pickups and progression

An eye-level camera must not make a key beneath the hero, a door behind
them or a transporter destination inactive. First-person item touches
therefore use the existing body contact tests without the rendered-view
gate. Keys, inventory caps, quest flags, item state, collision shapes,
health, power duration and exit requirements still apply. A first-person
transporter permits its paired destination without requiring it in view.
Classic touches retain their original visibility checks. Falling items
use the original gameplay camera bounds in personal views; equipment
cameras never participate. Monster activation, boss bounds, scripted
camera events and moving geometry still use the existing gameplay rigs.

## Validation and remaining work

The controls have regression tests for independent settings, Off restoring
classic facing, unchanged combat button bits and Robotron sticks, mouse
motion consumption and pane geometry. The real-data attack parser covers
all 16 PDATA files. Runtime checks and their latest results are recorded
in HANDOFF.md. The game runs were muted through tools/waitrun.sh; the
6 dB mix reduction is verified numerically, without a listening comparison.

This is not a complete campaign-clear claim. The existing level audit,
co-op combos and unfinished gameplay in STATUS.md still need their own
work. Three/four local pane geometry is tested, but runtime visual checks
so far use two local players. More class-specific equipment framing and
per-pane billboarding may benefit from player feedback. Use the new mode
with the same game build on every online peer.
