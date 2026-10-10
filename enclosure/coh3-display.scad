// Desk enclosure for the CoH3 display: a Seeed XIAO ESP32-C6 and a 0.96" SSD1306 module.
//
// Two parts, both printed without supports as they stand here:
//   shell  front with the screen window, sides, top and back; open underneath
//   base   the foot; carries the XIAO and snaps into the shell
//
// The display lies in a pocket behind the front wall. Nothing on the shell grips it: two
// ramps on the base press it against the wall, and two nubs carry it, once the case is
// closed. The case is as small as the two boards allow with the USB-C port at the back.
// Measure your display module before printing: see README.md.

/* [What to show] */
// assembly and section are for looking at; shell, base and fit_test are for printing.
part = "assembly"; // [assembly, section, shell, base, fit_test, clash]

/* [Display module] */
disp_w = 27.3;      // PCB width
disp_h = 27.8;      // PCB height (pins on the upper edge)
disp_t = 1.2;       // PCB thickness
glass_w = 26.7;
glass_h = 19.3;
// Front of the glass to the back of the PCB, measured at a side edge. This includes the
// tape the glass is stuck on with, and decides whether the module fits its rails.
disp_stack = 2.8;
glass_top = 5.3;    // PCB upper edge to glass upper edge
lit_w = 21.7;       // lit area, centred left to right
lit_h = 10.9;
lit_top = 2.1;      // glass upper edge to lit area upper edge

/* [XIAO ESP32-C6] */
xiao_l = 21.0;
xiao_w = 17.8;
xiao_t = 1.2;
usb_w = 9.0;        // USB-C shell
usb_h = 3.2;
usb_out = 1.5;      // how far it sticks out past the PCB
xiao_parts_h = 2.5; // tallest component on top

/* [Case] */
tilt = 45;          // how far the screen leans back, degrees
wall = 2.0;
plate_t = 1.6;      // thickness of the foot
corner_r = 2.0;     // rounding of the outside edges
side_gap = 1.6;     // between the display PCB and each side wall
fit = 0.25;         // clearance around the boards; raise if they do not go in
snap = 0.1;         // clearance of the base in the shell; raise if it is too tight
window_margin = 1.0;
flex_room = 1.0;    // below the display, for the fold of its flex cable
nose_h = 4.5;       // height of the blunt front edge, in place of a thin sharp toe

/* [Hidden] */
$fn = 48;
eps = 0.01;

// ---------------------------------------------------------------------------------------
// Geometry helpers. World: x across, y front to back, z up. "Face" coordinates describe
// things relative to the tilted front wall: n = into the case, v = up the slope.

function face_y(n, v) = n * cos(tilt) + v * sin(tilt);
function face_z(n, v) = -n * sin(tilt) + v * cos(tilt);

// Puts children built with y = n and z = v onto the front wall.
module on_face() {
    rotate([-tilt, 0, 0]) children();
}

disp_n1 = wall + disp_stack + 0.1;    // back of the display PCB
disp_n0 = disp_n1 - disp_t;           // front of the display PCB
rail_n = disp_n1 + 0.3;               // front of the lips that hold it
rail_t = 1.5;
width = disp_w + 2 * side_gap + 2 * wall;
disp_x0 = (width - disp_w) / 2;
// The display starts just above the foot and ends against the roof.
disp_v0 = (plate_t + flex_room + disp_n1 * sin(tilt)) / cos(tilt);
disp_v1 = disp_v0 + disp_h + fit;

height = face_z(disp_n0, disp_v1) + 0.1 + wall;
face_len = height / cos(tilt);

xiao_x = width / 2;
xiao_z0 = plate_t + 1.2;              // underside, lifted clear of solder joints
xiao_z1 = xiao_z0 + xiao_t;
// The XIAO goes as far under the leaning display as the parts on the display's back allow:
// neither the front of its cradle nor its own components may touch them.
cradle_h = xiao_z1 + 1.2;
xiao_y0 = (disp_n1 + 1.5) / cos(tilt) + 0.3
    + max(cradle_h * tan(tilt) + 1.2 + fit, (xiao_z1 + xiao_parts_h) * tan(tilt) - 0.5);
xiao_y1 = xiao_y0 + xiao_l;           // USB end of the PCB, against the back wall
depth = xiao_y1 + 0.1 + wall;

usb_gap = 0.4;
snap_rim_h = 4.2;
snap_z = plate_t + 2.8;
// The rims start behind the display rails, which lean back over the base.
snap_y0 = (rail_n + rail_t + (plate_t + snap_rim_h) * sin(tilt)) / cos(tilt) + 0.6;
snap_y1 = depth - wall - 1;

// The back wall is upright as far as the USB port needs it, then leans in towards the top
// of the display, which leaves room for the wires but little else.
back_knee_z = xiao_z1 + usb_h + usb_gap + 2.5;
roof_y1 = face_y(rail_n + rail_t, disp_v1) + 3 + wall;

// Side view of the whole case. `below` continues it under the foot.
module profile(below = 0) {
    nose_y = nose_h * tan(tilt);
    polygon([
        [nose_y, -below],
        [nose_y, nose_h],
        [face_len * sin(tilt), height],
        [roof_y1, height],
        [depth, back_knee_z],
        [depth, -below],
    ]);
}

// The outside: the profile across the full width, with every edge rounded except those of
// the foot, which stays flat and sharp so that it prints cleanly.
module body() {
    intersection() {
        minkowski() {
            across(corner_r, width - corner_r) offset(delta = -corner_r) profile(below = 10);
            sphere(r = corner_r, $fn = 32);
        }
        translate([-1, -20, 0]) cube([width + 2, depth + 40, height + 1]);
    }
}

// Extrudes a side-view shape across x from x0 to x1.
module across(x0, x1) {
    translate([x0, 0, 0]) rotate([90, 0, 90]) linear_extrude(x1 - x0) children();
}

// The hollow inside, optionally grown a little into the walls so that parts joined to
// them overlap instead of merely touching.
module cavity(grow = 0) {
    across(wall - grow, width - wall + grow) offset(delta = grow - wall) profile();
}

module mirror_x() {
    children();
    translate([width, 0, 0]) mirror([1, 0, 0]) children();
}

// ---------------------------------------------------------------------------------------
// Shell

module window() {
    w = lit_w + 2 * window_margin;
    h = lit_h + 2 * window_margin;
    v_top = disp_v0 + disp_h - glass_top - lit_top + window_margin;
    on_face() translate([width / 2, 0, v_top - h / 2]) {
        cube([w, 3 * wall, h], center = true);
        // Opens out towards the viewer, so the edge does not shade the screen.
        hull() {
            translate([0, wall / 2, 0]) cube([w, eps, h], center = true);
            translate([0, -eps, 0]) cube([w + wall, eps, h + wall], center = true);
        }
    }
}

// One of the two guides that keep the display from moving sideways. They end level with
// the back of its PCB, so the display can simply be laid in from behind.
module rail() {
    on_face() translate([0, 0, -10]) cube([disp_x0 - fit, disp_n1, face_len + 20]);
}

module usb_notch() {
    w = usb_w + 2 * usb_gap;
    translate([xiao_x - w / 2, depth - wall - 1, -1]) cube([w, wall + 2, xiao_z1 + usb_h + usb_gap + 1]);
}

module snap_groove() {
    // A touch above the ridge, so the ridge bears on its lower flank and pulls the shell down.
    translate([wall + 0.1, (snap_y0 + snap_y1) / 2, snap_z + 0.05]) rotate([90, 0, 0])
        cylinder(r = 0.75, h = snap_y1 - snap_y0 - 6, center = true);
}

module shell() {
    difference() {
        union() {
            difference() {
                body();
                cavity();
                // Open underneath.
                translate([0, 0, -wall - 1]) cavity();
            }
            intersection() {
                mirror_x() rail();
                cavity(0.5);
            }
        }
        // It stands on the base plate.
        translate([-1, -1, -10]) cube([width + 2, depth + 2, 10 + plate_t]);
        window();
        usb_notch();
        mirror_x() snap_groove();
        // A place to get a fingernail under the shell when opening it.
        translate([-1, depth / 2 - 5, plate_t - eps]) cube([wall, 10, 0.8]);
    }
}

// ---------------------------------------------------------------------------------------
// Base

module snap_rim() {
    x = wall + snap;
    // Solid, with a buttress on the inside: printed upright, a thin rim would snap off along
    // its layers. It does not bend; the shell's side walls spring out over the ridge instead.
    translate([x, snap_y0, plate_t - eps]) hull() {
        cube([2.0, snap_y1 - snap_y0, snap_rim_h]);
        cube([3.5, snap_y1 - snap_y0, 0.4]);
    }
    // The ridge that clicks into the groove of the shell.
    translate([x + 0.05, (snap_y0 + snap_y1) / 2, snap_z]) rotate([90, 0, 0])
        cylinder(r = 0.7, h = snap_y1 - snap_y0 - 8, center = true);
}

// One of the two ramps that hold the display against the front wall. They overlap the
// outer 2 mm of the back of its PCB and are solid wedges, so there is nothing to snap off.
module display_ramp() {
    x0 = wall + 0.4;
    back = (disp_n1 + 0.25) / cos(tilt);
    z1 = min(plate_t + 11, height - wall - 3);
    y0 = back + plate_t * tan(tilt);
    y1 = back + z1 * tan(tilt);
    translate([x0, 0, 0]) rotate([90, 0, 90]) linear_extrude(disp_x0 + 2.0 - x0)
        polygon([[y0, plate_t - eps], [y1, z1], [y1 + 1.5, z1], [y1 + 1.5, plate_t - eps]]);
}

// The display stands on two of these, which leaves the middle free for the fold of the
// flex cable.
module display_post() {
    y0 = face_y(disp_n0, disp_v0) - 0.6;
    y1 = face_y(disp_n1, disp_v0) + 0.3;
    translate([disp_x0 + 1.9, y0, plate_t - eps]) cube([3, y1 - y0, face_z(disp_n1, disp_v0) - 0.1 - plate_t]);
}

module xiao_cradle() {
    half = xiao_w / 2 + fit;
    rail_y0 = xiao_y0 - fit - 1.2;
    // Side rails, with a small bump each that the PCB clicks under.
    for (side = [-1, 1]) translate([xiao_x, 0, 0]) scale([side, 1, 1]) {
        translate([half, rail_y0, plate_t - eps]) cube([1.2, xiao_l - 3, cradle_h - plate_t]);
        translate([half + eps, xiao_y0 + 0.5, xiao_z1 + 0.15]) rotate([-90, 0, 0])
            linear_extrude(3) polygon([[0, 0], [-0.4, -0.4], [0, -1.0]]);
    }
    // Stop at the antenna end.
    translate([xiao_x - half - 1.2, rail_y0, plate_t - eps]) cube([2 * half + 2.4, 1.2, xiao_z1 - plate_t]);
    // The PCB rests on these two, away from the pad rows and their solder joints.
    translate([xiao_x - 5, xiao_y0 - fit - eps, plate_t - eps]) cube([10, 2.5, xiao_z0 - plate_t]);
    translate([xiao_x - 4, xiao_y1 - 3, plate_t - eps]) cube([8, 3 + 0.1, xiao_z0 - plate_t]);
    // Fills the part of the shell's USB opening that is below the connector.
    w = usb_w + 2 * usb_gap - 2 * snap;
    translate([xiao_x - w / 2, depth - wall - eps, plate_t - eps]) cube([w, wall, xiao_z1 - 0.3 - plate_t]);
}

module base() {
    intersection() {
        body();
        translate([-1, -1, 0]) cube([width + 2, depth + 2, plate_t]);
    }
    mirror_x() snap_rim();
    mirror_x() display_post();
    mirror_x() display_ramp();
    xiao_cradle();
}

// ---------------------------------------------------------------------------------------
// The boards, for looking at and for the clash check.

module display_board() {
    on_face() {
        color("darkgreen") translate([disp_x0, disp_n0, disp_v0]) cube([disp_w, disp_t, disp_h]);
        color("dimgray") translate([(width - glass_w) / 2, wall + 0.05, disp_v0 + disp_h - glass_top - glass_h])
            cube([glass_w, disp_n0 - wall - 0.05, glass_h]);
        // Components on the back.
        color("black") translate([disp_x0 + 3, disp_n1, disp_v0 + 2]) cube([disp_w - 6, 1.5, disp_h - 4]);
        color("deepskyblue")
            translate([(width - lit_w) / 2, wall + 0.02, disp_v0 + disp_h - glass_top - lit_top - lit_h])
            cube([lit_w, 0.05, lit_h]);
    }
}

module xiao_board() {
    color("darkgreen") translate([xiao_x - xiao_w / 2, xiao_y0, xiao_z0]) cube([xiao_w, xiao_l, xiao_t]);
    color("silver") translate([xiao_x - usb_w / 2, xiao_y1 - 6, xiao_z1]) cube([usb_w, 6 + usb_out, usb_h]);
    color("black") translate([xiao_x - xiao_w / 2 + 1, xiao_y0 + 0.5, xiao_z1])
        cube([xiao_w - 2, xiao_l - 7.5, xiao_parts_h]);
}

module boards() {
    display_board();
    xiao_board();
}

// ---------------------------------------------------------------------------------------

module assembly() {
    color("gainsboro") shell();
    color("slategray") base();
    boards();
}

if (part == "assembly") {
    assembly();
} else if (part == "section") {
    // Cut open a little left of the middle, so one display post and one cradle rail show.
    difference() {
        assembly();
        translate([-1, -1, -1]) cube([disp_x0 + 3.5 + 1, depth + 2, height + 2]);
    }
} else if (part == "shell") {
    translate([0, 0, -plate_t]) shell();
} else if (part == "base") {
    base();
} else if (part == "fit_test") {
    // The lowest 12 mm of the shell, and the whole base: a quick print to check that both
    // boards fit and that the two parts snap together, before printing the real shell.
    translate([0, 0, -plate_t]) intersection() {
        shell();
        translate([-1, -1, 0]) cube([width + 2, depth + 2, plate_t + 12]);
    }
    translate([width + 5, 0, 0]) base();
} else if (part == "clash") {
    // Must come out empty: no board touches the printed parts, and the two parts do not
    // run into each other.
    intersection() {
        union() {
            shell();
            base();
        }
        boards();
    }
    intersection() {
        shell();
        base();
    }
}

// Closing the case: the base, carrying the XIAO, moves straight up into the shell, which
// holds the display. With `part = "motion"` this shows what is in the way when the base is
// still `lift` mm short of home. Only the snap ridges may touch the side walls.
lift = 5;
if (part == "motion") {
    intersection() {
        union() {
            shell();
            display_board();
        }
        translate([0, 0, -lift]) union() {
            base();
            xiao_board();
        }
    }
}
