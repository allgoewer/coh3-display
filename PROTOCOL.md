# Display protocol

The tracker (host) sends the players to show to the display over the ESP32-C6's USB serial
port. Text lines, fields separated by a tab, each line ended by `\n`.

```
M <mode> <map>
P <faction> <elo> <wins> <losses> <streak> <country> <name>
E
```

| line | meaning |
|---|---|
| `M` | starts a frame and discards any unfinished one. `mode` is `1v1`..`4v4`; the display no longer shows it (since firmware 0.3.2), it stays in the frame so every firmware reads every app's frames. `map` is the map's title as the display should show it, at most 32 chars, made safe like `name` below. |
| `P` | one player, at most 8 per frame. The display gives each one its own page, in this order. |
| `E` | ends the frame. The display redraws and answers `OK\n`, or `ERR display\n` if the board cannot reach its OLED over I2C. |
| `I` | idle: there is no match worth showing. A frame of its own, answered like `E`. The display stops showing the match and falls back to its idle screen. |

The display is an OLED, which wears where it is lit, so it only shows a match while one is
current:

- The host sends the match while it is being played and for 5 minutes after the player left
  it; before and after that it sends `I`.
- A display that has heard nothing for 30 seconds goes idle on its own (the host is gone).
- Idle is a small tank (a Tiger or a Sherman) at the lowest brightness, driving across the screen below the top 16
  rows on a different row each time (the yellow part of the panel stays dark). After power-on and after a change of
  its WiFi state the display shows a status screen for 30 seconds first.

The host only sends the opposing team. When the local player is not in the match (a replay),
it sends everyone.

Player fields:

- `faction`: `A` american, `B` british, `D` dak, `G` german, `?` anything else.
- `elo`, `wins`, `losses`, `streak`: integers for the match's mode and the player's faction, or
  `-` when unknown (unrated, AI, API down). `streak` is negative for a losing streak.
- `country`: two-letter code, or `-`.
- `name`: at most 24 chars, UTF-8. Control characters are removed and characters outside
  Latin-1 are replaced by `?`, because that is what the display font covers.

The host repeats the current frame every 5 seconds, so the display recovers on its own after
a reset or replug. Lines longer than 128 bytes and unknown line types are ignored by the display.

## Board management (serial only)

| line | reply | meaning |
|---|---|---|
| `S` | `STATUS fw=<version> wifi=<state> [ip=<address>]` | what the board runs. `state` is `unset` (no credentials), `off` (credentials unusable), `connecting` or `up`. Firmware before 0.2.0 leaves out `fw=`. |
| `W <ssid> <password>` | `OK`, then the board restarts | stores WiFi credentials in flash (tab-separated like everything else). An empty password means an open network; `W` alone wipes them. `ERR wifi` if they are too long or flash could not be written. |

The PC app treats a board that does not answer `S` as having no display firmware and offers
to install the firmware it carries. Credentials live at the start of the `nvs` partition,
which installing firmware does not touch, so they survive updates.

## Over WiFi

The same frames travel over UDP once the display has joined the network (credentials are
set with the `W` line above).

| direction | port | payload |
|---|---|---|
| display -> broadcast | 42424 | beacon `COH3DISPLAY 1 <mac>\n`, every 2 s |
| PC -> broadcast | 42425 | probe `COH3DISPLAY? 1\n`, every 2 s; a display answers with its beacon, sent to the probe's source |
| PC -> display | 42425 | one complete frame (`M`, `P`..., `E`) in a single datagram |
| display -> PC | frame's source port | `OK\n` or `ERR display\n` |

- Discovery works either way round. The probe exists because a PC firewall that blocks
  unsolicited traffic drops the display's broadcast but lets the answer to a probe through.
- The PC takes the display's address from the beacon's source; nothing is configured.
- Frames are resent every 5 seconds as on serial. A display silent for 10 seconds is forgotten.
- There is no authentication: anything on the LAN can send frames to the display.

Host side: `src/display_link.rs` (serial), `src/display_link/udp.rs`, `src/firmware.rs`.
Device side: `firmware/src/protocol.rs`, `firmware/src/serial.rs`, `firmware/src/net.rs`,
`firmware/src/credentials.rs`.
