# Enclosure

A desk wedge for the XIAO ESP32-C6 and the 0.96" SSD1306 module. The screen leans back 45
degrees; the USB-C cable leaves at the back. 34.5 mm wide, 39 mm deep, 25.5 mm tall, with
rounded outside edges and no screws. It is about as small as the two boards allow with the
USB port at the back: the XIAO lies under the leaning display.

| File | |
|---|---|
| `coh3-display.scad` | the model; every dimension is a named parameter at the top |
| `stl/shell.stl` | front with the window, sides, top, back |
| `stl/base.stl` | the foot, with the XIAO cradle |
| `stl/fit_test.stl` | a quick print to check the fit first |
| `preview/` | rendered views |

## 1. Measure your display first

The XIAO's dimensions are fixed, but 0.96" display modules differ between sellers by up to a
millimetre. The model assumes the values below. Check them with a caliper and change the
parameters in the `.scad` file if yours differ by more than about 0.3 mm.

| Parameter | Assumed | What to measure |
|---|---|---|
| `disp_w`, `disp_h` | 27.3 x 27.8 mm | PCB width and height (pins on the upper edge) |
| `disp_t` | 1.2 mm | PCB thickness |
| `glass_w`, `glass_h` | 26.7 x 19.3 mm | the glass panel |
| `disp_stack` | 2.8 mm | front of the glass to the back of the PCB, at a side edge |
| `glass_top` | 5.3 mm | from the PCB's upper edge down to the glass's upper edge |
| `lit_top` | 2.1 mm | from the glass's upper edge down to the first lit row |
| `lit_w`, `lit_h` | 21.7 x 10.9 mm | the lit area (show a full-white or the match screen to see it) |

`glass_top` and `lit_top` decide where the window sits, so those two matter most.

The yellow rows are expected on the same side as the pins. If your module has them on the
opposite side, the picture would be upside down in this case.

## 2. Print

- 0.2 mm layers, 3 perimeters, 15-20 % infill. PLA or PETG. No supports.
- **Orientation: as the STL files are.** The shell stands on its open underside, the base lies flat.
- The front wall and the display rails rise at 45 degrees, which is the usual limit for
  printing without supports; good part cooling helps. The top of the window is a 24 mm bridge.
- `tilt` is a parameter. A steeper screen (say 30) prints more easily and makes the case
  taller and shorter; the rest of the model follows.

Print `fit_test.stl` first (about 25 minutes). It is the lowest 12 mm of the shell plus the
whole base:

- The XIAO should drop between the cradle's rails and click under the two small bumps.
- The ring should push down onto the base and click on both sides, and come off again when
  you pull a side wall outwards.
- Lay the display into the ring against the sloped wall, glass down: its lower end should
  sit between the two side guides without force.

| If | change | to |
|---|---|---|
| a board is tight or does not go in | `fit` | 0.35 |
| a board rattles | `fit` | 0.15 |
| the base will not snap into the shell | `snap` | 0.2 |
| the base is still loose | `snap` | 0.05 |

Then export again: in OpenSCAD set `part` to `shell`, `base` or `fit_test`, press F6, and
export as STL. Or from a terminal:

```powershell
& "C:\Program Files\OpenSCAD\openscad.com" -o stl\shell.stl -D 'part=\"shell\"' coh3-display.scad
```

## 3. Wire

Four wires, about 50 mm each, soldered directly (no pin headers; they do not fit):

| Display | XIAO |
|---|---|
| GND | GND |
| VCC | 3V3 |
| SCL | D5 |
| SDA | D4 |

- Solder to the **back** of the display's pads, so nothing sticks out towards the front wall,
  and lead the wires off sideways: there are only about 3 mm behind the display's upper edge.
- On the XIAO, solder from the top and cut the wire ends flush underneath; it rests 1.2 mm above the base.

## 4. Assemble

1. Press the XIAO into the cradle on the base, USB-C towards the tab at the back edge.
2. Hold the shell upside down with the window facing the table. Lay the display into it
   from behind: glass against the sloped wall, between the two side guides, pin edge towards
   the roof. Nothing clips it; it just lies there.
3. Keeping the shell like that, push the base straight in until both sides click. The two
   ramps on the base now press the display against the wall, and it stands on two small nubs.

To open it again, put a fingernail into the slot at the bottom of the left side wall and
pull that wall outwards a little.

## What has and has not been checked

- The model exports as a valid solid. A collision check of both boards against the case, and
  of the shell against the base, comes out empty (`part = "clash"`).
- Closing the case is simulated too (`part = "motion"`): on the way in, the base touches the
  shell only at the two snap ridges.
- **It has not been printed.** The display dimensions are typical values, not measurements of
  your module, and how firmly the snaps hold depends on your printer and material. That is
  what the fit test is for.
